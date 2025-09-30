use bigdecimal::{BigDecimal, ToPrimitive};
use std::str::FromStr;
use sqlx::{postgres::{PgTypeInfo, PgHasArrayType}, Type, Postgres, Encode, Decode};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use serde::{Serializer, Deserializer};
use serde::{Deserialize, Serialize};

/// Rounding mode supported (extendable later via feature flag / env)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode { HalfUp }

// Current global policy (future: load from env once)
const CURRENT_ROUNDING: RoundingMode = RoundingMode::HalfUp;

/// Apply configured rounding to scale=2 using Half-Up (away from zero on .5).
fn round_scale_2(value: &BigDecimal) -> BigDecimal {
    match CURRENT_ROUNDING {
        RoundingMode::HalfUp => half_up(value, 2),
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
}
