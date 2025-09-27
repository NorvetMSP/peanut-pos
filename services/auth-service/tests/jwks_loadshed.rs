use anyhow::Result;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{routing::get, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use common_auth::config::JwtConfig;
use common_auth::error::AuthError;
use common_auth::jwks::JwksFetcher;
use common_auth::verifier::JwtVerifier;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::rand_core::OsRng;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::Serialize;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Serialize)]
struct TokenClaims<'a> {
    sub: &'a str,
    tid: &'a str,
    roles: &'a [&'a str],
    iss: &'a str,
    aud: &'a str,
    exp: i64,
    iat: i64,
    jti: &'a str,
}

#[tokio::test(flavor = "multi_thread")]
async fn jwks_refresh_handles_load_shedding() -> Result<()> {
    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
    let public_key = private_key.to_public_key();
    let modulus = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
    let exponent = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
    let private_pem = private_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)?
        .to_string();

    let attempts = Arc::new(AtomicUsize::new(0));
    let jwks_body = json!({
        "keys": [
            {
                "kid": "test-key",
                "kty": "RSA",
                "alg": "RS256",
                "n": modulus,
                "e": exponent
            }
        ]
    });

    let router = Router::new().route(
        "/jwks",
        get({
            let attempts = attempts.clone();
            move || {
                let attempts = attempts.clone();
                let jwks_body = jwks_body.clone();
                async move {
                    let step = attempts.fetch_add(1, Ordering::SeqCst);
                    match step {
                        0 => (StatusCode::OK, Json(jwks_body.clone())).into_response(),
                        1 => StatusCode::BAD_GATEWAY.into_response(),
                        2 => (StatusCode::OK, axum::body::Body::from("not json")).into_response(),
                        _ => (StatusCode::OK, Json(jwks_body.clone())).into_response(),
                    }
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let make_service = router.into_make_service();
    let server = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, make_service).await {
            eprintln!("JWKS server error: {err}");
        }
    });
    let jwks_url = format!("http://{}/jwks", addr);

    let fetcher = JwksFetcher::new(jwks_url);

    let jwt_config = JwtConfig::new("test-issuer", "test-audience");
    let verifier = JwtVerifier::builder(jwt_config)
        .with_jwks_fetcher(fetcher)
        .build()
        .await?;

    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    let user_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();
    let now = Utc::now();
    let jti = Uuid::new_v4();
    let sub = user_id.to_string();
    let tid = tenant_id.to_string();
    let jti_str = jti.to_string();
    let roles = ["admin"];
    let claims = TokenClaims {
        sub: &sub,
        tid: &tid,
        roles: &roles,
        iss: "test-issuer",
        aud: "test-audience",
        exp: (now + ChronoDuration::minutes(15)).timestamp(),
        iat: now.timestamp(),
        jti: &jti_str,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key".to_string());
    let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes())?;
    let token = encode(&header, &claims, &encoding_key)?;

    verifier.verify(&token)?;

    match verifier.refresh_jwks().await {
        Ok(_) => panic!("expected JWKS refresh failure"),
        Err(AuthError::JwksFetch(_)) => (),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    verifier.verify(&token)?;

    match verifier.refresh_jwks().await {
        Ok(_) => panic!("expected JWKS timeout failure"),
        Err(AuthError::JwksFetch(_) | AuthError::JwksDecode(_)) => (),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
    assert!(attempts.load(Ordering::SeqCst) >= 3);
    verifier.verify(&token)?;

    server.abort();
    Ok(())
}
