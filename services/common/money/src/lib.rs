use bigdecimal::BigDecimal;
use bigdecimal::ToPrimitive;
use serde::{Deserialize, Serialize};

/// Normalize a monetary value to 2 decimal places (banker's rounding not applied; BigDecimal uses plain rounding when reducing scale)
pub fn normalize_scale(value: &BigDecimal) -> BigDecimal {
    // Set scale to 2 using with_scale, which truncates/extends with zeros.
    value.with_scale(2)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedMoney(BigDecimal);

impl NormalizedMoney {
    pub fn new(raw: BigDecimal) -> Self {
        Self(normalize_scale(&raw))
    }
    pub fn inner(&self) -> &BigDecimal { &self.0 }
}

impl From<BigDecimal> for NormalizedMoney {
    fn from(value: BigDecimal) -> Self { Self::new(value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bigdecimal::BigDecimal;    
    #[test]
    fn test_normalize() {
        let v = BigDecimal::parse_bytes(b"12.3456", 10).unwrap();
        assert_eq!(normalize_scale(&v).to_string(), "12.34");
    }
    #[test]
    fn test_nearly_equal() {
        let a = BigDecimal::parse_bytes(b"10.001", 10).unwrap();
        let b = BigDecimal::parse_bytes(b"10.009", 10).unwrap();
        assert!(nearly_equal(&a, &b, 1)); // 1 cent tolerance
    }
}
