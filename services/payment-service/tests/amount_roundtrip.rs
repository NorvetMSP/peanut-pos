use bigdecimal::BigDecimal;use serde::{Deserialize,Serialize};use common_money::normalize_scale;
#[derive(Serialize,Deserialize,Debug)]
struct AmountPayload{amount:BigDecimal}
#[test]
fn amount_roundtrip_normalizes(){
 let original=BigDecimal::parse_bytes(b"5.678",10).unwrap();
 let payload=AmountPayload{amount:original.clone()};
 let json=serde_json::to_string(&payload).unwrap();
 let de:AmountPayload=serde_json::from_str(&json).unwrap();
 // Half-up rounding: 5.678 -> 5.68
 assert_eq!(normalize_scale(&de.amount).to_string(),"5.68");
}
