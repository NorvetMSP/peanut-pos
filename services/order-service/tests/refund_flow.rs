use bigdecimal::BigDecimal;use common_money::{Money, normalize_scale};

#[test]
fn refund_flow_arithmetic_sanity() {
    let unit_price_raw = BigDecimal::parse_bytes(b"12.345", 10).unwrap();
    let unit_price = Money::new(unit_price_raw.clone()); // rounds half-up to 12.35
    assert_eq!(unit_price.inner().to_string(), "12.35");
    let qty = 3i32;
    let extended_raw = unit_price_raw * BigDecimal::from(qty); // 37.035
    let extended = Money::new(extended_raw.clone()); // half-up => 37.04
    assert_eq!(extended.inner().to_string(), "37.04");
    // Aggregate by rounding each line vs total raw rounding could differ; ensure normalization matches policy
    let expected_total = normalize_scale(&(unit_price.inner() * BigDecimal::from(qty))); // 12.35 * 3 = 37.05 -> stays 37.05
    // Demonstrate difference between rounding per-unit vs rounding at aggregate: we intentionally round units individually.
    // Here we just assert normalization of multiplication of rounded unit price.
    assert_eq!(expected_total.to_string(), "37.05");
}
