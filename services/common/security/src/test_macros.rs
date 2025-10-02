//! Shared test helper macro for constructing security headers quickly.
//! Usage: test_request_headers!(req, roles="admin support", tenant="<uuid>", user="<uuid>");
#[macro_export]
macro_rules! test_request_headers {
    ($req:expr, roles=$roles:expr, tenant=$tenant:expr, user=$user:expr) => {{
        let h = $req.headers_mut();
        h.insert("X-Tenant-ID", ::axum::http::HeaderValue::from_str($tenant).unwrap());
        h.insert("X-Roles", ::axum::http::HeaderValue::from_str($roles).unwrap());
        h.insert("X-User-ID", ::axum::http::HeaderValue::from_str($user).unwrap());
    }};
    ($req:expr, roles=$roles:expr, tenant=$tenant:expr) => {{
        let h = $req.headers_mut();
        h.insert("X-Tenant-ID", ::axum::http::HeaderValue::from_str($tenant).unwrap());
        h.insert("X-Roles", ::axum::http::HeaderValue::from_str($roles).unwrap());
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macro_compiles() {
        let mut req = ::axum::http::Request::builder().uri("/").body(::axum::body::Body::empty()).unwrap();
        test_request_headers!(req, roles="admin", tenant="11111111-1111-1111-1111-111111111111", user="22222222-2222-2222-2222-222222222222");
        assert!(req.headers().get("X-Roles").is_some());
    }
}