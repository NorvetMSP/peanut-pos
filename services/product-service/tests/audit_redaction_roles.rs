use product_service::audit_handlers::redact_event_fields;

fn paths(list: &[&str]) -> Vec<Vec<String>> { list.iter().map(|p| p.split('.').map(|s| s.to_string()).collect()).collect() }

#[test]
fn non_privileged_removal() {
    let payload = serde_json::json!({"sensitive":{"secret":"abc","other":1},"keep":5});
    let meta = serde_json::json!({"audit":{"email":"u@example.com"}});
    let (p,m,fields,count) = redact_event_fields(payload, meta, &paths(&["sensitive.secret","audit.email"]), false);
    assert_eq!(count, 2);
    assert_eq!(fields.len(), 2);
    assert!(p["sensitive"].get("secret").is_none());
    assert!(m["audit"].get("email").is_none());
}

#[test]
fn non_privileged_masking() {
    let payload = serde_json::json!({"sensitive":{"secret":"abc"}});
    let meta = serde_json::json!({"audit":{"email":"u@example.com"}});
    let (p,m,_fields,count) = redact_event_fields(payload, meta, &paths(&["sensitive.secret","audit.email"]), true);
    assert_eq!(count, 2);
    assert_eq!(p["sensitive"]["secret"], serde_json::json!("****"));
    assert_eq!(m["audit"]["email"], serde_json::json!("****"));
}

#[test]
fn path_not_found_noop() {
    let payload = serde_json::json!({"root":1});
    let meta = serde_json::json!({});
    let (_p,_m,fields,count) = redact_event_fields(payload, meta, &paths(&["missing.field"]), true);
    assert_eq!(count, 0);
    assert!(fields.is_empty());
}
