use axum::{http::{HeaderValue, StatusCode}, middleware::Next};
use axum::body::Body;
use axum::response::Response;
use hmac::{Hmac, Mac};
use sha2::{Sha256, Digest};
use subtle::ConstantTimeEq;
use crate::AppState;
use tracing::warn;

// Webhook signature verification middleware with HMAC, timestamp skew and nonce replay protection
pub async fn verify_webhook(req: axum::http::Request<Body>, next: Next) -> Response {
    // Only guard webhook paths (placeholder: match on "/webhooks/")
    let is_webhook = req.uri().path().starts_with("/webhooks/");
    if !is_webhook {
        return next.run(req).await;
    }

    // Extract headers
    let sig: String = req
        .headers()
        .get("X-Signature")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let ts: String = req
        .headers()
        .get("X-Timestamp")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let nonce: String = req
        .headers()
        .get("X-Nonce")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    if sig.is_empty() || ts.is_empty() || nonce.is_empty() {
        let mut resp = axum::http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("missing signature"))
            .unwrap();
        resp.headers_mut()
            .insert("X-Error-Code", HeaderValue::from_static("sig_missing"));
        return resp;
    }

    // Buffer body (consume and rebuild request)
    let (mut parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        // 1MB cap
        Ok(b) => b,
        Err(_) => {
            let mut resp = axum::http::Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("malformed"))
                .unwrap();
            resp.headers_mut()
                .insert("X-Error-Code", HeaderValue::from_static("malformed"));
            return resp;
        }
    };

    // Build canonical string: ts, nonce, body_sha256
    let body_hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    let canonical = format!("ts:{}\nnonce:{}\nbody_sha256:{}", ts, nonce, body_hash);

    // Secret lookup (placeholder: from env)
    let secret = std::env::var("WEBHOOK_ACTIVE_SECRET").unwrap_or_default();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(canonical.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    let provided = sig.strip_prefix("sha256=").unwrap_or(sig.as_str());
    let eq = ConstantTimeEq::ct_eq(expected.as_bytes(), provided.as_bytes()).unwrap_u8();
    if eq != 1 {
        let mut resp = axum::http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("signature mismatch"))
            .unwrap();
        resp.headers_mut()
            .insert("X-Error-Code", HeaderValue::from_static("sig_mismatch"));
        return resp;
    }

    // Timestamp skew validation and nonce replay protection (when DB is configured)
    // Parse timestamp as unix epoch seconds
    let ts_num: i64 = match ts.parse() {
        Ok(v) => v,
        Err(_) => {
            let mut resp = axum::http::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::from("invalid timestamp"))
                .unwrap();
            resp.headers_mut()
                .insert("X-Error-Code", HeaderValue::from_static("sig_ts_invalid"));
            return resp;
        }
    };
    let now = chrono::Utc::now().timestamp();
    let max_skew = std::env::var("WEBHOOK_MAX_SKEW_SECS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(300);
    if (now - ts_num).unsigned_abs() as i64 > max_skew {
        let mut resp = axum::http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("timestamp skew"))
            .unwrap();
        resp.headers_mut()
            .insert("X-Error-Code", HeaderValue::from_static("sig_skew"));
        return resp;
    }

    // Access AppState to reach DB (if configured)
    // Note: state is stored in request extensions by axum's with_state
    let provider_hdr = parts
        .headers
        .get("X-Provider")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if let Some(state) = parts.extensions.get::<AppState>().cloned() {
        if let Some(pool) = state.db.clone() {
            // Attempt to insert nonce; if it already exists, this is a replay.
            // Use INSERT ... ON CONFLICT DO NOTHING RETURNING 1 to detect insert vs conflict atomically.
            match sqlx::query_scalar::<_, i32>(
                "INSERT INTO webhook_nonces (nonce, provider) VALUES ($1, $2) ON CONFLICT (nonce) DO NOTHING RETURNING 1",
            )
            .bind(&nonce)
            .bind(provider_hdr.as_deref())
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(_)) => { /* inserted ok, proceed */ }
                Ok(None) => {
                    let mut resp = axum::http::Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::from("nonce replay"))
                        .unwrap();
                    resp.headers_mut()
                        .insert("X-Error-Code", HeaderValue::from_static("sig_replay"));
                    return resp;
                }
                Err(err) => {
                    warn!(error = %err, "failed nonce insert; allowing for resilience");
                    // Fall through: if DB errors, prefer to allow webhook rather than drop it silently
                }
            }
        } else {
            // No DB configured: skip nonce replay enforcement
        }
    }

    if let Ok(cl) = HeaderValue::from_str(&bytes.len().to_string()) {
        parts
            .headers
            .insert(axum::http::header::CONTENT_LENGTH, cl);
    }
    let req = axum::http::Request::from_parts(parts, Body::from(bytes));
    next.run(req).await
}
