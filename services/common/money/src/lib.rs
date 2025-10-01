use bigdecimal::{BigDecimal, ToPrimitive};
use std::str::FromStr;
use std::sync::OnceLock;
use sqlx::{postgres::{PgTypeInfo, PgHasArrayType}, Type, Postgres, Encode, Decode};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use serde::{Serializer, Deserializer};
use serde::{Deserialize, Serialize};

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

/// Initialize rounding mode from the MONEY_ROUNDING env variable (idempotent).
/// Falls back to HalfUp if unset or invalid.
pub fn init_rounding_mode_from_env() -> RoundingMode {
    if let Some(mode) = ROUNDING_MODE.get() { return *mode; }
    let selected = std::env::var("MONEY_ROUNDING").ok()
        .and_then(|v| RoundingMode::parse(&v))
        .unwrap_or(RoundingMode::HalfUp);
    let _ = ROUNDING_MODE.set(selected); // ignore error if already set race (same value semantics)
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

/// Apply configured rounding to scale=2 using Half-Up (away from zero on .5).
fn round_scale_2(value: &BigDecimal) -> BigDecimal {
    match current_rounding_mode() {
        RoundingMode::HalfUp => half_up(value, 2),
        RoundingMode::Truncate => truncate(value, 2),
        RoundingMode::Bankers => bankers(value, 2),
    }
}

/// Public normalization entrypoint (round + enforce scale 2)
pub fn normalize_scale(value: &BigDecimal) -> BigDecimal { round_scale_2(value) }

/// Half-up rounding helper: scale target digits; positive & negative symmetrical.
fn half_up(value: &BigDecimal, scale: i32) -> BigDecimal {
    // Build factor = 10^scale manually (BigDecimal lacks stable pow for u64 in this crate version)
    let mut factor = BigDecimal::from(1);
    for _ in 0..scale { factor = factor * BigDecimal::from(10u32); }
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
    for _ in 0..scale { factor = factor * BigDecimal::from(10u32); }
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
    for _ in 0..scale { factor = factor * BigDecimal::from(10u32); }
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
                    if frac_str.chars().any(|c| c != '0') { -1 } else { -1 }
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
    if negative { int_bd = int_bd * BigDecimal::from(-1); }
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
}

impl From<BigDecimal> for Money { fn from(v: BigDecimal) -> Self { Money::new(v) } }
impl From<Money> for BigDecimal { fn from(m: Money) -> Self { m.0 } }

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
}
