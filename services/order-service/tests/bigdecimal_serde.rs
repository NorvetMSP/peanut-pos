use uuid::Uuid;
use bigdecimal::BigDecimal;
use common_money::{Money, normalize_scale};

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct TestOrderItemMoney {
    pub product_id: Uuid,
    pub product_name: Option<String>,
    pub quantity: i32,
    pub unit_price: Money,
    pub line_total: Money,
}

#[test]
fn test_money_order_item_roundtrip_half_up() {
    // 12.345 * 2 => unit_price rounds to 12.35, line_total computed externally should round accordingly
    let unit_price_raw = BigDecimal::parse_bytes(b"12.345", 10).unwrap();
    let line_total_raw = &unit_price_raw * BigDecimal::from(2); // 24.69 (already 2 decimals after rounding each? we round aggregate)
    let json = serde_json::json!({
        "product_id": "00000000-0000-0000-0000-000000000001",
        "product_name": "Test Product",
        "quantity": 2,
        "unit_price": unit_price_raw.to_string(),
        "line_total": line_total_raw.to_string()
    }).to_string();
    let item: TestOrderItemMoney = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(item.unit_price.inner().to_string(), "12.35");
    assert_eq!(item.line_total.inner().to_string(), normalize_scale(&line_total_raw).to_string());
    let back = serde_json::to_string(&item).expect("serialize");
    assert!(back.contains("12.35"));
}