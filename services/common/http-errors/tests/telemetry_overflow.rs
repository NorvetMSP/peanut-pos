use common_http_errors::test_helpers::{simulate_error_code, distinct_gauge, overflow_count};

#[test]
fn distinct_and_overflow_tracking() {
    // Simulate fewer than limit
    for i in 0..5 {
        simulate_error_code(&format!("code_{}", i));
    }
    assert!(distinct_gauge() >= 5, "expected at least 5 distinct codes");
    let before_overflow = overflow_count();

    // Drive up to (and past) guard; limit defined as 40
    for i in 5..50 { // push beyond 40
        simulate_error_code(&format!("code_{}", i));
    }
    assert!(distinct_gauge() as usize <= 40, "distinct gauge should be capped at guard (<=40), got {}", distinct_gauge());
    assert!(overflow_count() > before_overflow, "expected overflow counter to increment");
}
