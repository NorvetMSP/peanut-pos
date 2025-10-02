// Crate name is product-service (hyphen) which is not a valid Rust identifier for an extern crate.
// Tests compiled within the same package can use the implicit crate root `crate`.

use product_service::audit_handlers::view_redactions_count;

#[test]
fn view_redaction_counter_initial_zero() {
    assert_eq!(view_redactions_count(), 0, "counter should start at zero");
}
