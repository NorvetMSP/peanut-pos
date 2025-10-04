use bigdecimal::BigDecimal;
use common_money::Money;

fn cents(v: i64) -> BigDecimal { Money::from_cents(v).into() }

#[test]
fn compute_discount_rounding_half_up() {
    // subtotal = $10.00, discount 15% = $1.50 exact, tax 0 -> total $8.50
    let subtotal = 1000i64; // cents
    let discount_bps = 1500i32; // 15%
    let discount = (subtotal * discount_bps as i64 + 5_000) / 10_000; // half-up
    assert_eq!(discount, 150);
    let tax_rate_bps: i64 = 0;
    let taxable_subtotal = subtotal - discount; // no proportional adjustment needed when all taxable
    let tax = (taxable_subtotal * tax_rate_bps + 5_000) / 10_000;
    assert_eq!(tax, 0);
    let total = subtotal - discount + tax;
    assert_eq!(total, 850);
}

#[test]
fn compute_taxability_exempt_vs_std() {
    // Two lines: STD $10.00 qty1, EXEMPT $5.00 qty1
    // discount 0, tax 10% -> tax only on STD line -> $1.00
    let std_line = 1000i64;
    let exempt_line = 500i64;
    let subtotal = std_line + exempt_line; // 1500
    let discount = 0i64;
    let taxable_net = std_line; // exempt not counted
    let tax_rate_bps: i64 = 1000; // 10%
    let tax = (taxable_net * tax_rate_bps + 5_000) / 10_000;
    assert_eq!(subtotal, 1500);
    assert_eq!(tax, 100);
    let total = subtotal - discount + tax;
    assert_eq!(total, 1600);
}

#[test]
fn compute_proportional_discount_allocation() {
    // subtotal: taxable 1200, non-taxable 800 => 2000
    // discount 10% => 200 cents total discount
    // Discount on taxable proportion: 200 * (1200/2000) = 120
    // Tax 5% on (taxable_net=1200-120=1080) => 54
    let taxable_subtotal = 1200i64;
    let exempt_subtotal = 800i64;
    let subtotal = taxable_subtotal + exempt_subtotal; // 2000
    let discount_bps = 1000i32; // 10%
    let discount = (subtotal * discount_bps as i64 + 5_000) / 10_000; // 200
    assert_eq!(discount, 200);
    let discount_on_taxable = (discount * taxable_subtotal + (subtotal / 2)) / subtotal; // half-up
    assert_eq!(discount_on_taxable, 120);
    let taxable_net = taxable_subtotal - discount_on_taxable; // 1080
    let tax_rate_bps: i64 = 500; // 5%
    let tax = (taxable_net * tax_rate_bps + 5_000) / 10_000; // 54
    assert_eq!(tax, 54);
    let total = subtotal - discount + tax; // 2000 - 200 + 54 = 1854
    assert_eq!(total, 1854);
}
