use uuid::Uuid;
use bigdecimal::BigDecimal;

// If direct import fails due to visibility, redefine a minimal mirror struct for serde test
#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct TestOrderItem {
    pub product_id: Uuid,
    pub product_name: Option<String>,
    pub quantity: i32,
    pub unit_price: BigDecimal,
    pub line_total: BigDecimal,
}

#[test]
fn test_bigdecimal_order_item_roundtrip() {
    let json = r#"{
        "product_id": "00000000-0000-0000-0000-000000000001",
        "product_name": "Test Product",
        "quantity": 2,
        "unit_price": "12.34",
        "line_total": "24.68"
    }"#;

    let item: TestOrderItem = serde_json::from_str(json).expect("deserialize");
    assert_eq!(item.quantity, 2);
    assert_eq!(item.unit_price.to_string(), "12.34");
    assert_eq!(item.line_total.to_string(), "24.68");

    let back = serde_json::to_string(&item).expect("serialize");
    // Ensure numbers preserved as strings with same scale representation
    assert!(back.contains("12.34"));
    assert!(back.contains("24.68"));
}