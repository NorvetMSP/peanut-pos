use bigdecimal::BigDecimal;use serde::{Deserialize,Serialize};use common_money::normalize_scale;
#[derive(Serialize,Deserialize,Debug)]
struct PricePayload{price:BigDecimal}
#[test]
fn price_roundtrip_normalizes(){
 let original=BigDecimal::parse_bytes(b"19.999",10).unwrap();
 let payload=PricePayload{price:original.clone()};
 let json=serde_json::to_string(&payload).unwrap();
 let de:PricePayload=serde_json::from_str(&json).unwrap();
 // with_scale(2) truncates instead of rounding; 19.999 -> 19.99
 assert_eq!(normalize_scale(&de.price).to_string(),"19.99");
}
