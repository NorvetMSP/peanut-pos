use auth_service::user_handlers::{login_user, logout_user, refresh_session};
use auth_service::{tokens::TokenConfig, AppState};
use auth_service::metrics::AuthMetrics;
use auth_service::notifications::KafkaProducer;
use auth_service::mfa_handlers::{begin_mfa_enrollment, verify_mfa_enrollment};
use axum::{Router, routing::{post, get}, http::{Request, header::{SET_COOKIE, COOKIE}, StatusCode}, body::Body};
use common_auth::{JwtConfig, JwtVerifier};
use jsonwebtoken::DecodingKey;
use reqwest::Client;
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPublicKey, pkcs8::EncodePrivateKey};
use rand_core::OsRng;
use serde_json::json;
use std::sync::Arc;
use tower::util::ServiceExt;

mod support;
use support::{seed_test_user, default_auth_config, RecordingKafkaProducer, TestDatabase};

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(feature = "integration"), ignore = "enable with --features integration (requires Postgres)")]
async fn session_flow_basic_refresh_and_logout() -> anyhow::Result<()> {
    let Some(db) = TestDatabase::setup().await? else { return Ok(()); };
    let pool = db.pool_clone();

    let seeded = seed_test_user(&pool, "session").await?;

    // Minimal signer / verifier bootstrap mirroring smoke test logic.
    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
    let private_pem = private_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)?.to_string();
    let public_pem = private_key.to_public_key().to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)?.to_string();

    let jwt_config = JwtConfig::new("test-issuer", "test-audience");
    let token_config = TokenConfig { issuer: "test-issuer".into(), audience: "test-audience".into(), access_ttl_seconds: 300, refresh_ttl_seconds: 900 };
    let token_signer = auth_service::tokens::TokenSigner::new(pool.clone(), token_config, Some(&private_pem)).await?;
    let jwks = token_signer.jwks().await?;
    let mut verifier_builder = JwtVerifier::builder(jwt_config);
    if jwks.is_empty() { verifier_builder = verifier_builder.with_rsa_pem("local-dev", public_pem.as_bytes())?; } else { for key in &jwks { verifier_builder = verifier_builder.with_decoding_key(key.kid.clone(), DecodingKey::from_rsa_components(&key.n, &key.e).expect("invalid jwk")); } }
    let verifier = verifier_builder.build().await?;

    let kafka_recorder = RecordingKafkaProducer::default();
    let kafka_producer: Arc<dyn KafkaProducer> = Arc::new(kafka_recorder);
    let http_client = Client::builder().build()?;
    let config = default_auth_config();
    let state = AppState { db: pool.clone(), jwt_verifier: Arc::new(verifier), token_signer: Arc::new(token_signer), config: Arc::new(config), kafka_producer, http_client, metrics: Arc::new(AuthMetrics::new()?) };

    let app = Router::new()
        .route("/login", post(login_user))
        .route("/session", get(refresh_session))
        .route("/logout", post(logout_user))
        .route("/mfa/enroll", post(begin_mfa_enrollment))
        .route("/mfa/verify", post(verify_mfa_enrollment))
        .with_state(state.clone());

    // 1. Login
    let login_body = json!({
        "email": seeded.email,
        "password": seeded.password,
        "tenant_id": seeded.tenant_id,
        "mfa_code": null,
        "device_fingerprint": null
    }).to_string();
    let login_req = Request::builder().method("POST").uri("/login").header("content-type", "application/json").body(Body::from(login_body))?;
    let login_resp = app.clone().oneshot(login_req).await?;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let set_cookie = login_resp.headers().get(SET_COOKIE).ok_or_else(|| anyhow::anyhow!("missing refresh cookie"))?.to_str()?;
    assert!(set_cookie.contains("novapos_refresh"));
    let cookie_pair = set_cookie.split(';').next().unwrap().to_string();

    // 2. Refresh (should succeed)
    let refresh_req = Request::builder().method("GET").uri("/session").header(COOKIE, &cookie_pair).body(Body::empty())?;
    let refresh_resp = app.clone().oneshot(refresh_req).await?;
    assert_eq!(refresh_resp.status(), StatusCode::OK);

    // 3. Logout
    let logout_req = Request::builder().method("POST").uri("/logout").header(COOKIE, &cookie_pair).body(Body::empty())?;
    let logout_resp = app.clone().oneshot(logout_req).await?;
    assert_eq!(logout_resp.status(), StatusCode::NO_CONTENT);

    // 4. Refresh after logout (should be 401)
    let refresh_again_req = Request::builder().method("GET").uri("/session").header(COOKIE, &cookie_pair).body(Body::empty())?;
    let refresh_again_resp = app.clone().oneshot(refresh_again_req).await?;
    assert_eq!(refresh_again_resp.status(), StatusCode::UNAUTHORIZED);

    db.teardown().await?;
    Ok(())
}
