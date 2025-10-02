use serde_json::Value;

/// Redaction result containing whether a field was redacted and collected field path.
pub fn apply_redaction(root: &mut Value, path: &[String], include_redacted: bool) -> bool {
    if path.is_empty() { return false; }
    recurse(root, path, include_redacted)
}

fn recurse(current: &mut Value, path: &[String], include_redacted: bool) -> bool {
    if path.is_empty() { return false; }
    let seg = &path[0];
    if path.len() == 1 {
        if let Value::Object(map) = current {
            if map.contains_key(seg) {
                if include_redacted {
                    map.insert(seg.clone(), Value::String("****".into()));
                } else {
                    map.remove(seg);
                }
                return true;
            }
        }
        return false;
    }
    if let Value::Object(map) = current {
        if let Some(child) = map.get_mut(seg) {
            return recurse(child, &path[1..], include_redacted);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_existing_path_removes() {
        let mut v: Value = serde_json::json!({"a":{"b":{"c":123,"keep":1}}});
        let ok = apply_redaction(&mut v, &["a".into(),"b".into(),"c".into()], false);
        assert!(ok);
        assert_eq!(v["a"]["b"].get("c"), None);
    }
    #[test]
    fn redacts_existing_path_masks() {
        let mut v: Value = serde_json::json!({"a":{"b":{"c":"secret"}}});
        let ok = apply_redaction(&mut v, &["a".into(),"b".into(),"c".into()], true);
        assert!(ok);
        assert_eq!(v["a"]["b"]["c"], Value::String("****".into()));
    }
    #[test]
    fn non_existent_path_noop() {
        let mut v: Value = serde_json::json!({"a":1});
        let ok = apply_redaction(&mut v, &["missing".into()], true);
        assert!(!ok);
    }
}
