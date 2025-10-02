use product_service::view_redaction::apply_redaction;
use serde_json::json;

#[test]
fn non_existent_path_returns_false() {
    let mut v = json!({"payload":{"a":1}});
    assert!(!apply_redaction(&mut v, &["payload".into(),"missing".into()], true));
}

#[test]
fn mask_and_remove_variants() {
    let mut v1 = json!({"meta":{"secret":"value"}});
    assert!(apply_redaction(&mut v1, &["meta".into(),"secret".into()], true));
    assert_eq!(v1["meta"]["secret"], json!("****"));

    let mut v2 = json!({"meta":{"secret":"value"}});
    assert!(apply_redaction(&mut v2, &["meta".into(),"secret".into()], false));
    assert!(v2["meta"].get("secret").is_none());
}
