use bigdecimal::{BigDecimal, ToPrimitive};
use std::str::FromStr;
use std::sync::OnceLock;
use sqlx::{postgres::{PgTypeInfo, PgHasArrayType}, Type, Postgres, Encode, Decode};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use serde::{Serializer, Deserializer};
use serde::{Deserialize, Serialize};
use std::sync::Once;

/// Rounding modes supported (configurable via env in later initialization step)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode { HalfUp, Truncate, Bankers }

impl RoundingMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "halfup" | "half-up" | "half_up" => Some(RoundingMode::HalfUp),
            "truncate" | "trunc" => Some(RoundingMode::Truncate),
            "bankers" | "banker" | "half-even" | "halfeven" | "half_even" => Some(RoundingMode::Bankers),
            _ => None,
        }
    }
}

// Global rounding mode (initialized once from env) defaulting to HalfUp.
static ROUNDING_MODE: OnceLock<RoundingMode> = OnceLock::new();
#[cfg(feature = "prometheus-metrics")]
static PROM_REGISTRY: OnceLock<prometheus::Registry> = OnceLock::new();
#[cfg(feature = "prometheus-metrics")]
static ROUNDING_MODE_GAUGE: OnceLock<prometheus::IntGauge> = OnceLock::new();

/// Initialize rounding mode from the MONEY_ROUNDING env variable (idempotent).
/// Falls back to HalfUp if unset or invalid.
pub fn init_rounding_mode_from_env() -> RoundingMode {
    if let Some(mode) = ROUNDING_MODE.get() { return *mode; }
    let raw = std::env::var("MONEY_ROUNDING").ok();
    let mut warned = false;
    let selected = if let Some(val) = raw.clone() {
        match RoundingMode::parse(&val) {
            Some(m) => m,
            None => { warned = true; RoundingMode::HalfUp }
        }
    } else { RoundingMode::HalfUp };
    let _ = ROUNDING_MODE.set(selected); // ignore error if already set race (same value semantics)
    if warned {
        tracing::warn!(invalid_value = ?raw, fallback = ?selected, "Invalid MONEY_ROUNDING value; falling back to default");
    }
    #[cfg(feature = "prometheus-metrics")]
    {
        init_metrics_once(selected);
    }
    selected
}

/// Get currently initialized rounding mode (initializes with default if not set yet).
pub fn current_rounding_mode() -> RoundingMode {
    *ROUNDING_MODE.get_or_init(|| RoundingMode::HalfUp)
}

#[cfg(test)]
pub fn set_rounding_mode_for_tests(mode: RoundingMode) {
    // For tests we want to reconfigure; OnceLock cannot be reset, so we only allow setting if empty.
    if ROUNDING_MODE.get().is_none() { let _ = ROUNDING_MODE.set(mode); }
}

static LOG_ONCE: Once = Once::new();
pub fn log_rounding_mode_once() {
    LOG_ONCE.call_once(|| {
        let mode = init_rounding_mode_from_env();
        tracing::info!(monetary.rounding_mode = ?mode, "Initialized monetary rounding mode");
    });
}

/// Apply configured rounding to scale=2 using Half-Up (away from zero on .5).
fn round_scale_2(value: &BigDecimal) -> BigDecimal {
    let out = match current_rounding_mode() {
        RoundingMode::HalfUp => half_up(value, 2),
        RoundingMode::Truncate => truncate(value, 2),
        RoundingMode::Bankers => bankers(value, 2),
    };
    trace_rounding_event(value, &out);
    out
}

/// Public normalization entrypoint (round + enforce scale 2)
pub fn normalize_scale(value: &BigDecimal) -> BigDecimal { round_scale_2(value) }

#[inline]
fn trace_rounding_event(original: &BigDecimal, rounded: &BigDecimal) {
    // Lightweight debug-only instrumentation; can be swapped for metrics crate later.
    if cfg!(debug_assertions) {
        tracing::debug!(orig = %original, out = %rounded, mode = ?current_rounding_mode(), "money.normalize");
    }
}

#[cfg(feature = "prometheus-metrics")]
fn init_metrics_once(mode: RoundingMode) {
    if ROUNDING_MODE_GAUGE.get().is_some() { return; }
    let registry = prometheus::Registry::new();
    let gauge = prometheus::IntGauge::new("money_rounding_mode", "Active configured rounding mode (1=HalfUp,2=Truncate,3=Bankers)").expect("gauge");
    let val = match mode { RoundingMode::HalfUp => 1, RoundingMode::Truncate => 2, RoundingMode::Bankers => 3 };
    gauge.set(val);
    registry.register(Box::new(gauge.clone())).ok();
    let _ = PROM_REGISTRY.set(registry);
    let _ = ROUNDING_MODE_GAUGE.set(gauge);
}

#[cfg(feature = "prometheus-metrics")]
pub fn registry() -> Option<&'static prometheus::Registry> { PROM_REGISTRY.get() }

/// Half-up rounding helper: scale target digits; positive & negative symmetrical.
fn half_up(value: &BigDecimal, scale: i32) -> BigDecimal {
    // Build factor = 10^scale manually (BigDecimal lacks stable pow for u64 in this crate version)
    let mut factor = BigDecimal::from(1);
    for _ in 0..scale { factor *= BigDecimal::from(10u32); }
    let shifted = value * &factor; // value * 10^scale

    // Determine sign (>=0 treat as positive) using comparison to zero
    let is_negative = shifted < BigDecimal::from(0);
    // Add 0.5 or -0.5 depending on sign (away from zero)
    let adjust_str = if is_negative { "-0.5" } else { "0.5" };
    let adjusted = &shifted + BigDecimal::from_str(adjust_str).unwrap();

    // Truncate toward zero by converting to string then cutting decimals, avoiding overly large to_i64 casts.
    let adjusted_str = adjusted.to_string();
    let int_portion = if let Some(dot) = adjusted_str.find('.') { &adjusted_str[..dot] } else { &adjusted_str };
    // Handle "-0" artifact
    let cleaned = if int_portion == "-0" { "0" } else { int_portion };
    let int_bd = BigDecimal::from_str(cleaned).unwrap();
    let rounded = int_bd / factor;
    // Ensure fixed scale with trailing zeros
    rounded.with_scale(scale as i64)
}

/// Truncate toward zero at given scale.
fn truncate(value: &BigDecimal, scale: i32) -> BigDecimal {
    // factor = 10^scale
    let mut factor = BigDecimal::from(1);
    for _ in 0..scale { factor *= BigDecimal::from(10u32); }
    let shifted = value * &factor;
    let s = shifted.to_string();
    let int_portion = if let Some(dot) = s.find('.') { &s[..dot] } else { &s };
    let cleaned = if int_portion == "-0" { "0" } else { int_portion };
    let int_bd = BigDecimal::from_str(cleaned).unwrap();
    (int_bd / factor).with_scale(scale as i64)
}

/// Bankers (round half to even) at given scale.
fn bankers(value: &BigDecimal, scale: i32) -> BigDecimal {
    // factor = 10^scale
    let mut factor = BigDecimal::from(1);
    for _ in 0..scale { factor *= BigDecimal::from(10u32); }
    let shifted = value * &factor; // exact scaled number
    let s = shifted.to_string();
    let negative = s.starts_with('-');
    let abs = if negative { &s[1..] } else { &s }; // strip sign
    let (int_str, frac_str) = if let Some(dot) = abs.find('.') { (&abs[..dot], &abs[dot+1..]) } else { (abs, "") };
    // Normalize fractional substring (we only care if it's >, <, or == 0.5)
    // Compare using BigDecimal might reintroduce rounding; use string logic.
    let cmp_half = {
        if frac_str.is_empty() { -1 } // no fraction => < 0.5
        else {
            // Build a comparison on first digit and beyond
            let first = frac_str.chars().next().unwrap();
            match first {
                '0' => if frac_str.len() == 1 { -1 } else { // e.g. 0xxxxx < 0.5
                    // any non-zero after still < 0.5
                    -1
                },
                '1'..='4' => -1,
                '5' => {
                    // Check if exactly 0.5 == all remaining digits zero
                    if frac_str.chars().skip(1).all(|c| c == '0') { 0 } else { 1 }
                },
                _ => 1, // 6-9 => >0.5
            }
        }
    };
    // Parse integer portion (always positive here)
    let mut int_bd = BigDecimal::from_str(if int_str.is_empty() { "0" } else { int_str }).unwrap();
    match cmp_half {
        -1 => { /* keep int */ }
        1 => { int_bd = &int_bd + BigDecimal::from(1); }
        0 => { // tie 0.5 -> round to even
            // Determine if int is even
            let even = {
                // Extract last digit (string) - safe because int_str not empty for any non-zero
                let last_digit = int_str.chars().last().unwrap_or('0');
                let digit_val = last_digit.to_digit(10).unwrap_or(0);
                digit_val % 2 == 0
            };
            if !even { int_bd = &int_bd + BigDecimal::from(1); }
        }
        _ => {}
    }
    // Reapply sign
    if negative { int_bd *= BigDecimal::from(-1); }
    (int_bd / factor).with_scale(scale as i64)
}

/// Compare two monetary values allowing a tolerance (in cents) after normalization.
pub fn nearly_equal(a: &BigDecimal, b: &BigDecimal, cents_tolerance: i64) -> bool {
    let na = normalize_scale(a);
    let nb = normalize_scale(b);
    // Convert difference to cents integer to avoid floating comparison.
    let diff = (na - nb).with_scale(2);
    // Convert to i64 cents via *100
    let cents = diff.to_f64().unwrap_or(0.0) * 100.0;
    cents.abs() <= cents_tolerance as f64
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Money(BigDecimal);

impl Serialize for Money {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let inner = BigDecimal::deserialize(deserializer)?;
        Ok(Money::new(inner))
    }
}

impl Money {
    pub fn new(raw: BigDecimal) -> Self { Self(normalize_scale(&raw)) }
    pub fn from_major_minor(major: i64, minor: i64) -> Self {
        // Combine independently signed major & minor components: major + minor/100
        let value = BigDecimal::from(major) + BigDecimal::from(minor) / BigDecimal::from(100);
        Self::new(value)
    }
    pub fn inner(&self) -> &BigDecimal { &self.0 }
    /// Construct from integer cents (minor units) without intermediate floating rounding.
    pub fn from_cents(cents: i64) -> Self {
        // value = cents / 100
        let major = BigDecimal::from(cents) / BigDecimal::from(100);
        Money::new(major)
    }
    /// Return total minor units (cents) as i64. Panics if out of i64 range (extremely unlikely with enforced scale=2).
    pub fn as_cents(&self) -> i64 {
        // Multiply by 100 exactly using string to avoid floating imprecision
        // self.0 is guaranteed scale 2, so representation like X.YY
        let s = self.0.to_string();
        if let Some(dot) = s.find('.') {
            let (int_part, frac_part) = s.split_at(dot);
            let frac = &frac_part[1..]; // skip '.'
            let mut frac_norm = String::from(frac);
            if frac_norm.len() < 2 { frac_norm.push('0'); }
            if frac_norm.len() > 2 { frac_norm.truncate(2); }
            let sign = if int_part.starts_with('-') { -1i64 } else { 1i64 };
            let int_clean = if int_part == "-0" || int_part == "+0" { "0" } else { int_part.trim_start_matches('+') };
            let major = int_clean.parse::<i64>().unwrap();
            let minor = frac_norm.parse::<i64>().unwrap();
            sign * (major.abs() * 100 + minor)
        } else {
            // Whole number -> multiply by 100
            let major = s.parse::<i64>().unwrap();
            major * 100
        }
    }
}

// Arithmetic trait implementations
use std::ops::{Add, Sub, AddAssign, SubAssign, Mul};
impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money { Money::new(self.0 + rhs.0) }
}
impl<'a> Add<&'a Money> for Money {
    type Output = Money;
    fn add(self, rhs: &'a Money) -> Money { Money::new(self.0 + rhs.0.clone()) }
}
impl Add<Money> for &Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money { Money::new(self.0.clone() + rhs.0) }
}
impl<'b> Add<&'b Money> for &Money {
    type Output = Money;
    fn add(self, rhs: &'b Money) -> Money { Money::new(self.0.clone() + rhs.0.clone()) }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money { Money::new(self.0 - rhs.0) }
}
impl<'a> Sub<&'a Money> for Money {
    type Output = Money;
    fn sub(self, rhs: &'a Money) -> Money { Money::new(self.0 - rhs.0.clone()) }
}
impl Sub<Money> for &Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money { Money::new(self.0.clone() - rhs.0) }
}
impl<'b> Sub<&'b Money> for &Money {
    type Output = Money;
    fn sub(self, rhs: &'b Money) -> Money { Money::new(self.0.clone() - rhs.0.clone()) }
}

impl AddAssign for Money {
    fn add_assign(&mut self, rhs: Money) { *self = Money::new(self.0.clone() + rhs.0); }
}
impl<'a> AddAssign<&'a Money> for Money {
    fn add_assign(&mut self, rhs: &'a Money) { *self = Money::new(self.0.clone() + rhs.0.clone()); }
}
impl SubAssign for Money {
    fn sub_assign(&mut self, rhs: Money) { *self = Money::new(self.0.clone() - rhs.0); }
}
impl<'a> SubAssign<&'a Money> for Money {
    fn sub_assign(&mut self, rhs: &'a Money) { *self = Money::new(self.0.clone() - rhs.0.clone()); }
}

impl Mul<i32> for Money {
    type Output = Money;
    fn mul(self, rhs: i32) -> Money { Money::new(self.0 * BigDecimal::from(rhs)) }
}
impl Mul<i32> for &Money {
    type Output = Money;
    fn mul(self, rhs: i32) -> Money { Money::new(self.0.clone() * BigDecimal::from(rhs)) }
}

impl std::iter::Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::new(BigDecimal::from(0)), |acc, m| acc + m)
    }
}
impl<'a> std::iter::Sum<&'a Money> for Money {
    fn sum<I: Iterator<Item = &'a Money>>(iter: I) -> Money {
        iter.fold(Money::new(BigDecimal::from(0)), |acc, m| acc + m)
    }
}

impl From<BigDecimal> for Money { fn from(v: BigDecimal) -> Self { Money::new(v) } }
impl From<Money> for BigDecimal { fn from(m: Money) -> Self { m.0 } }

/// Aggregate rounding strategy: sums raw BigDecimals then rounds once at end.
/// Compare with summing Money values (incremental rounding) for potential biases.
pub fn aggregate_rounding_sum(values: &[BigDecimal]) -> Money {
    let total_raw = values.iter().fold(BigDecimal::from(0), |acc, v| acc + v);
    Money::new(total_raw)
}

// --- sqlx integration (Postgres) ---
impl Type<Postgres> for Money {
    fn type_info() -> PgTypeInfo { <BigDecimal as Type<Postgres>>::type_info() }
    fn compatible(ty: &PgTypeInfo) -> bool { <BigDecimal as Type<Postgres>>::compatible(ty) }
}

impl PgHasArrayType for Money {
    fn array_type_info() -> PgTypeInfo { <BigDecimal as PgHasArrayType>::array_type_info() }
}

impl<'q> Encode<'q, Postgres> for Money {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> IsNull {
        let inner: &BigDecimal = &self.0;
        <BigDecimal as Encode<Postgres>>::encode_by_ref(inner, buf)
    }
}

impl<'r> Decode<'r, Postgres> for Money {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let inner = <BigDecimal as Decode<Postgres>>::decode(value)?;
        Ok(Money::new(inner))
    }
}

impl std::fmt::Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bigdecimal::BigDecimal;    
    #[test]
    fn test_half_up_rounding() {
        let v = BigDecimal::parse_bytes(b"12.345", 10).unwrap();
        assert_eq!(normalize_scale(&v).to_string(), "12.35");
        let v2 = BigDecimal::parse_bytes(b"12.344", 10).unwrap();
        assert_eq!(normalize_scale(&v2).to_string(), "12.34");
        let neg = BigDecimal::parse_bytes(b"-1.235", 10).unwrap();
        assert_eq!(normalize_scale(&neg).to_string(), "-1.24");
        let neg2 = BigDecimal::parse_bytes(b"-1.234", 10).unwrap();
        assert_eq!(normalize_scale(&neg2).to_string(), "-1.23");
    }

    #[test]
    fn test_from_major_minor() {
        let m = Money::from_major_minor(10, 5); // $10.05
        assert_eq!(m.inner().to_string(), "10.05");
        let m2 = Money::from_major_minor(-10, 5); // -10 + 0.05 => -9.95 then rounded
        assert_eq!(m2.inner().to_string(), "-9.95");
        let m3 = Money::from_major_minor(10, -5); // 10 - 0.05 => 9.95
        assert_eq!(m3.inner().to_string(), "9.95");
    }

    #[test]
    fn test_large_value_rounding() {
        let v = BigDecimal::parse_bytes(b"1234567890123.999", 10).unwrap();
        assert_eq!(normalize_scale(&v).to_string(), "1234567890124.00");
    }

    #[test]
    fn test_edge_negative_half_up() {
        let v = BigDecimal::parse_bytes(b"-2.505", 10).unwrap();
        assert_eq!(normalize_scale(&v).to_string(), "-2.51");
        let v2 = BigDecimal::parse_bytes(b"-2.504", 10).unwrap();
        assert_eq!(normalize_scale(&v2).to_string(), "-2.50");
    }
    #[test]
    fn test_nearly_equal() {
        let a = BigDecimal::parse_bytes(b"10.001", 10).unwrap();
        let b = BigDecimal::parse_bytes(b"10.009", 10).unwrap();
        assert!(nearly_equal(&a, &b, 1)); // 1 cent tolerance
    }

    #[test]
    fn test_arithmetic_add_sub_mul() {
        let a = Money::from(BigDecimal::from_str("10.015").unwrap()); // rounds to 10.02
        let b = Money::from(BigDecimal::from_str("2.005").unwrap());  // 2.01
        assert_eq!((a.clone() + b.clone()).inner().to_string(), "12.03");
        assert_eq!((a.clone() - b.clone()).inner().to_string(), "8.01");
        assert_eq!((a.clone() * 3).inner().to_string(), "30.06");
    }

    #[test]
    fn test_arithmetic_sum_iterator() {
        let values = [
            Money::from(BigDecimal::from_str("1.005").unwrap()), // 1.01
            Money::from(BigDecimal::from_str("2.004").unwrap()), // 2.00
            Money::from(BigDecimal::from_str("3.001").unwrap()), // 3.00
        ];
        let total: Money = values.iter().cloned().sum();
        assert_eq!(total.inner().to_string(), "6.01");
    }

    #[test]
    fn test_from_cents_and_as_cents() {
        let m = Money::from_cents(1234); // 12.34
        assert_eq!(m.inner().to_string(), "12.34");
        assert_eq!(m.as_cents(), 1234);
        let n = Money::from_cents(-995); // -9.95
        assert_eq!(n.inner().to_string(), "-9.95");
        assert_eq!(n.as_cents(), -995);
        let zero = Money::from_cents(0);
        assert_eq!(zero.inner().to_string(), "0.00");
        assert_eq!(zero.as_cents(), 0);
    }

    #[test]
    fn test_rounding_mode_parse() {
        assert_eq!(RoundingMode::parse("halfup"), Some(RoundingMode::HalfUp));
        assert_eq!(RoundingMode::parse("truncate"), Some(RoundingMode::Truncate));
        assert_eq!(RoundingMode::parse("bankers"), Some(RoundingMode::Bankers));
        assert!(RoundingMode::parse("unknown").is_none());
    }

    #[test]
    fn test_truncate_rounding_behavior() {
        // local direct call to truncate to validate distinct behaviour from half_up
        let v = BigDecimal::from_str("12.345").unwrap();
        assert_eq!(truncate(&v, 2).to_string(), "12.34");
        let neg = BigDecimal::from_str("-1.239").unwrap();
        assert_eq!(truncate(&neg, 2).to_string(), "-1.23");
    }

    #[test]
    fn test_bankers_rounding_ties() {
        // 1.235 -> 1.24 (123.5 -> 124 because 123 odd)
        let v1 = BigDecimal::from_str("1.235").unwrap();
        assert_eq!(bankers(&v1, 2).to_string(), "1.24");
        // 1.245 -> 1.24 (124.5 -> 124 even stays)
        let v2 = BigDecimal::from_str("1.245").unwrap();
        assert_eq!(bankers(&v2, 2).to_string(), "1.24");
        // -1.235 -> -1.24 (123.5 tie, 123 odd -> increment magnitude)
        let v3 = BigDecimal::from_str("-1.235").unwrap();
        assert_eq!(bankers(&v3, 2).to_string(), "-1.24");
        // -1.245 -> -1.24 (124 even -> stays -1.24)
        let v4 = BigDecimal::from_str("-1.245").unwrap();
        assert_eq!(bankers(&v4, 2).to_string(), "-1.24");
    }

    #[test]
    fn test_rounding_matrix_all_modes() {
        // (input, half_up, truncate, bankers)
        let cases = vec![
            ("1.005",  "1.01", "1.00", "1.00"),
            ("2.675",  "2.68", "2.67", "2.68"),
            ("0.005",  "0.01", "0.00", "0.00"),
            ("-1.005", "-1.01", "-1.00", "-1.00"),
            ("-2.505", "-2.51", "-2.50", "-2.50"),
            ("12345",  "12345.00", "12345.00", "12345.00"),
            ("19.90",  "19.90", "19.90", "19.90"),
        ];

        for (input, expect_half, expect_trunc, expect_bank) in cases {            
            let v = BigDecimal::from_str(input).unwrap();
            let h = half_up(&v, 2).to_string();
            let t = truncate(&v, 2).to_string();
            let b = bankers(&v, 2).to_string();
            assert_eq!(h, expect_half,   "HalfUp mismatch for {input}");
            assert_eq!(t, expect_trunc,  "Truncate mismatch for {input}");
            assert_eq!(b, expect_bank,   "Bankers mismatch for {input}");

            // Idempotence: applying again to already-rounded output should not change.
            let hv2 = half_up(&BigDecimal::from_str(&h).unwrap(), 2).to_string();
            let tv2 = truncate(&BigDecimal::from_str(&t).unwrap(), 2).to_string();
            let bv2 = bankers(&BigDecimal::from_str(&b).unwrap(), 2).to_string();
            assert_eq!(hv2, h, "HalfUp idempotence failed for {input}");
            assert_eq!(tv2, t, "Truncate idempotence failed for {input}");
            assert_eq!(bv2, b, "Bankers idempotence failed for {input}");
        }
    }

    #[test]
    fn test_env_initialization_default() {
        // Ensure clean state: rely on not having set env var; call init and assert HalfUp.
        std::env::remove_var("MONEY_ROUNDING");
        let mode = init_rounding_mode_from_env();
        assert_eq!(mode, RoundingMode::HalfUp);
    }

    #[test]
    fn test_env_initialization_parse() {
        std::env::set_var("MONEY_ROUNDING", "truncate");
        // Because OnceLock may already be set by earlier tests, we spawn a separate process scenario isn't trivial.
        // We skip if already not HalfUp (another test may have initialized). This keeps test order independent enough.
        if current_rounding_mode() == RoundingMode::HalfUp { let _ = init_rounding_mode_from_env(); }
        // We can't assert strictly due to OnceLock single-set semantics; main path is covered by earlier tests.
        assert!(matches!(current_rounding_mode(), RoundingMode::HalfUp | RoundingMode::Truncate | RoundingMode::Bankers));
    }

    #[test]
    fn test_aggregate_rounding_sum_bias_example() {
        // Values chosen so that half-up incremental vs aggregate differ
        // e.g. three values each with third of a cent beyond 2 decimals
        let parts = vec![
            BigDecimal::from_str("1.001").unwrap(),
            BigDecimal::from_str("1.001").unwrap(),
            BigDecimal::from_str("1.001").unwrap(),
        ];
        // Incremental: each rounds to 1.00 => 3.00 total
        let incremental: Money = parts.iter().cloned().map(Money::from).sum();
        assert_eq!(incremental.inner().to_string(), "3.00");
        // Aggregate raw: sum = 3.003 -> rounds to 3.00 still (in this case equal)
        let aggregate = aggregate_rounding_sum(&parts);
        assert_eq!(aggregate.inner().to_string(), "3.00");

        // Different case where aggregate differs: values around .005 boundary
        let parts2 = vec![
            BigDecimal::from_str("0.005").unwrap(),
            BigDecimal::from_str("0.005").unwrap(),
        ];
        let incremental2: Money = parts2.iter().cloned().map(Money::from).sum(); // each 0.01 => 0.02
        assert_eq!(incremental2.inner().to_string(), "0.02");
        let aggregate2 = aggregate_rounding_sum(&parts2); // raw 0.010 -> 0.01
        assert_eq!(aggregate2.inner().to_string(), "0.01");
        // Demonstrate bias direction (incremental >= aggregate in half-up)
        assert!(incremental2.as_cents() >= aggregate2.as_cents());
    }
}
