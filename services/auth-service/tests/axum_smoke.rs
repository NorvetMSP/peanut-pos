mod support;

use anyhow::{anyhow, Result};
use auth_service::metrics::AuthMetrics;
use auth_service::mfa::generate_totp_secret;
use auth_service::notifications::KafkaProducer;
use auth_service::tenant_handlers::{
    create_integration_key, create_tenant, list_integration_keys, list_tenants,
    revoke_integration_key,
};
use auth_service::tokens::{JwkKey, TokenConfig, TokenSigner};
use auth_service::user_handlers::{login_user, logout_user, refresh_session};
use auth_service::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{
    header::{COOKIE, SET_COOKIE},
    HeaderMap, Request, StatusCode,
};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use common_auth::{JwtConfig, JwtVerifier};
use http_body_util::BodyExt;
use rand_core::OsRng;
use reqwest::Client;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use serde::Serialize;
use serde_json::{json, Value};
use std::str;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use support::{
    current_totp_code, default_auth_config, seed_test_user, RecordingKafkaProducer, TestDatabase,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tower::util::ServiceExt;
use uuid::Uuid;

const ROOT_TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";

async fn health() -> &'static str {
    "ok"
}

async fn jwks_endpoint(State(state): State<AppState>) -> Result<Json<JwksResponse>, StatusCode> {
    let signer = state.token_signer.clone();
    match signer.jwks().await {
        Ok(keys) => Ok(Json(JwksResponse { keys })),
        Err(err) => {
            tracing::warn!(error = %err, "Unable to load JWKS");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn metrics_endpoint(State(state): State<AppState>) -> Response {
    state.metrics.render().expect("metrics render")
}

#[derive(Serialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires embedded Postgres binary download"]
async fn axum_smoke_tests_core_routes() -> Result<()> {
    let Some(db) = TestDatabase::setup().await? else {
        return Ok(());
    };
    let pool = db.pool_clone();

    let seeded = seed_test_user(&pool, "admin").await?;

    let (webhook_tx, mut webhook_rx) = mpsc::unbounded_channel::<(HeaderMap, Value)>();
    let webhook_listener = TcpListener::bind("127.0.0.1:0").await?;
    let webhook_addr = webhook_listener.local_addr()?;
    let webhook_fail = Arc::new(AtomicBool::new(true));
    let webhook_router = {
        let sender = webhook_tx.clone();
        let fail_once = webhook_fail.clone();
        Router::new().route(
            "/",
            post(move |headers: HeaderMap, Json(payload): Json<Value>| {
                let sender = sender.clone();
                let fail_once = fail_once.clone();
                async move {
                    if fail_once.swap(false, Ordering::SeqCst) {
                        StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        let _ = sender.send((headers, payload));
                        StatusCode::OK
                    }
                }
            }),
        )
    };
    let _webhook_handle = tokio::spawn(async move {
        let _ = axum::serve(webhook_listener, webhook_router).await;
    });
    let webhook_url = format!("http://{}/", webhook_addr);

    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
    let private_pem = private_key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)?
        .to_string();
    let public_pem = private_key
        .to_public_key()
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)?
        .to_string();

    let jwt_config = JwtConfig::new("test-issuer", "test-audience");
    let verifier = JwtVerifier::builder(jwt_config)
        .with_rsa_pem("local-test", public_pem.as_bytes())?
        .build()
        .await?;

    let token_config = TokenConfig {
        issuer: "test-issuer".to_string(),
        audience: "test-audience".to_string(),
        access_ttl_seconds: 900,
        refresh_ttl_seconds: 7200,
    };
    let token_signer = TokenSigner::new(pool.clone(), token_config, Some(&private_pem)).await?;

    let kafka_recorder = RecordingKafkaProducer::default();
    let kafka_producer: Arc<dyn KafkaProducer> = Arc::new(kafka_recorder.clone());

    let http_client = Client::builder().build()?;

    let mut config = default_auth_config();
    config.mfa_activity_topic = "security.test.mfa".to_string();
    config.required_roles.insert("manager".to_string());
    config.suspicious_webhook_url = Some(webhook_url.clone());
    config.suspicious_webhook_bearer = Some("test-token".to_string());
    let state = AppState {
        db: pool.clone(),
        jwt_verifier: Arc::new(verifier),
        token_signer: Arc::new(token_signer),
        config: Arc::new(config),
        kafka_producer,
        http_client,
        metrics: Arc::new(AuthMetrics::new()?),
    };

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(metrics_endpoint))
        .route("/jwks", get(jwks_endpoint))
        .route("/.well-known/jwks.json", get(jwks_endpoint))
        .route("/login", post(login_user))
        .route("/session", get(refresh_session))
        .route("/logout", post(logout_user))
        .route("/tenants", post(create_tenant).get(list_tenants))
        .route(
            "/tenants/:tenant_id/integration-keys",
            post(create_integration_key).get(list_integration_keys),
        )
        .route(
            "/integration-keys/:key_id/revoke",
            post(revoke_integration_key),
        )
        .with_state(state.clone());

    let response = app
        .clone()
        .oneshot(Request::builder().uri("/healthz").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let health_body = response.into_body().collect().await?.to_bytes();
    assert_eq!(health_body.as_ref(), b"ok");

    let login_body = json!({
        "email": seeded.email.clone(),
        "password": seeded.password.clone(),
        "tenant_id": seeded.tenant_id,
        "mfa_code": null,
        "device_fingerprint": null
    });
    let login_request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .body(Body::from(login_body.to_string()))?;
    let login_response = app.clone().oneshot(login_request).await?;
    assert_eq!(login_response.status(), StatusCode::OK);
    let cookies = login_response
        .headers()
        .get(SET_COOKIE)
        .ok_or_else(|| anyhow!("missing login cookie"))?
        .to_str()?;
    assert!(cookies.contains("novapos_refresh"));
    let cookie_pair = cookies
        .split(';')
        .next()
        .ok_or_else(|| anyhow!("invalid cookie format"))?
        .trim()
        .to_string();

    let metrics_response = app
        .clone()
        .oneshot(Request::builder().uri("/metrics").body(Body::empty())?)
        .await?;
    assert_eq!(metrics_response.status(), StatusCode::OK);
    let metrics_body = metrics_response.into_body().collect().await?.to_bytes();
    let metrics_text = str::from_utf8(metrics_body.as_ref())?;
    assert!(metrics_text.contains("auth_login_attempts_total"));
    assert!(metrics_text.contains("success"));

    kafka_recorder.fail_times("simulated kafka outage", 2);
    let mfa_user = seed_test_user(&pool, "manager").await?;
    let mfa_secret = generate_totp_secret();
    sqlx::query("UPDATE users SET mfa_secret = $1, mfa_enrolled_at = NOW(), mfa_pending_secret = NULL, mfa_failed_attempts = 0 WHERE id = $2")
        .bind(&mfa_secret)
        .bind(mfa_user.user_id)
        .execute(&pool)
        .await?;

    let missing_code_body = json!({
        "email": mfa_user.email.clone(),
        "password": mfa_user.password.clone(),
        "tenant_id": mfa_user.tenant_id,
        "device_fingerprint": "mfa-device"
    });
    let missing_code_request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.10")
        .body(Body::from(missing_code_body.to_string()))?;
    let missing_code_response = app.clone().oneshot(missing_code_request).await?;
    assert_eq!(missing_code_response.status(), StatusCode::UNAUTHORIZED);
    let missing_bytes = missing_code_response
        .into_body()
        .collect()
        .await?
        .to_bytes();
    let missing_json: Value = serde_json::from_slice(&missing_bytes)?;
    assert_eq!(missing_json["code"], Value::from("MFA_REQUIRED"));

    let invalid_code_body = json!({
        "email": mfa_user.email.clone(),
        "password": mfa_user.password.clone(),
        "tenant_id": mfa_user.tenant_id,
        "mfa_code": "000000",
        "device_fingerprint": "mfa-device"
    });
    let invalid_code_request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.10")
        .body(Body::from(invalid_code_body.to_string()))?;
    let invalid_code_response = app.clone().oneshot(invalid_code_request).await?;
    assert_eq!(invalid_code_response.status(), StatusCode::UNAUTHORIZED);
    let invalid_bytes = invalid_code_response
        .into_body()
        .collect()
        .await?
        .to_bytes();
    let invalid_json: Value = serde_json::from_slice(&invalid_bytes)?;
    assert_eq!(invalid_json["code"], Value::from("MFA_CODE_INVALID"));

    let (webhook_headers, webhook_payload) = webhook_rx
        .recv()
        .await
        .expect("expected webhook notification");
    assert_eq!(
        webhook_headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer test-token")
    );
    let webhook_text = webhook_payload["text"].as_str().unwrap_or("");
    assert!(webhook_text.contains("mfa.challenge.failed"));

    let valid_code = current_totp_code(&mfa_secret)?;
    let success_body = json!({
        "email": mfa_user.email.clone(),
        "password": mfa_user.password.clone(),
        "tenant_id": mfa_user.tenant_id,
        "mfa_code": valid_code,
        "device_fingerprint": "mfa-device"
    });
    let success_request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.10")
        .body(Body::from(success_body.to_string()))?;
    let success_response = app.clone().oneshot(success_request).await?;
    assert_eq!(success_response.status(), StatusCode::OK);

    let enroll_user = seed_test_user(&pool, "cashier").await?;
    let enroll_login_body = json!({
        "email": enroll_user.email.clone(),
        "password": enroll_user.password.clone(),
        "tenant_id": enroll_user.tenant_id,
        "device_fingerprint": "enroll-device"
    });
    let enroll_login_request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .body(Body::from(enroll_login_body.to_string()))?;
    let enroll_login_response = app.clone().oneshot(enroll_login_request).await?;
    assert_eq!(enroll_login_response.status(), StatusCode::OK);
    let enroll_login_bytes = enroll_login_response
        .into_body()
        .collect()
        .await?
        .to_bytes();
    let enroll_login_json: Value = serde_json::from_slice(&enroll_login_bytes)?;
    let access_token = enroll_login_json["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("missing access token"))?
        .to_string();
    let auth_header = format!("Bearer {}", access_token);

    let enroll_request = Request::builder()
        .method("POST")
        .uri("/mfa/enroll")
        .header("authorization", &auth_header)
        .body(Body::empty())?;
    let enroll_response = app.clone().oneshot(enroll_request).await?;
    assert_eq!(enroll_response.status(), StatusCode::OK);
    let enroll_bytes = enroll_response.into_body().collect().await?.to_bytes();
    let enroll_json: Value = serde_json::from_slice(&enroll_bytes)?;
    let enrollment_secret = enroll_json["secret"]
        .as_str()
        .ok_or_else(|| anyhow!("missing enrollment secret"))?
        .to_string();
    let verify_code = current_totp_code(&enrollment_secret)?;

    let verify_body = json!({ "code": verify_code });
    let verify_request = Request::builder()
        .method("POST")
        .uri("/mfa/verify")
        .header("authorization", &auth_header)
        .header("content-type", "application/json")
        .body(Body::from(verify_body.to_string()))?;
    let verify_response = app.clone().oneshot(verify_request).await?;
    assert_eq!(verify_response.status(), StatusCode::OK);
    let verify_json: Value =
        serde_json::from_slice(&verify_response.into_body().collect().await?.to_bytes())?;
    assert_eq!(verify_json["enabled"], Value::from(true));

    let jwks_response = app
        .clone()
        .oneshot(Request::builder().uri("/jwks").body(Body::empty())?)
        .await?;
    assert_eq!(jwks_response.status(), StatusCode::OK);
    let jwks_bytes = jwks_response.into_body().collect().await?.to_bytes();
    let jwks_json: Value = serde_json::from_slice(&jwks_bytes)?;
    assert_eq!(jwks_json["keys"].as_array().map(|arr| arr.len()), Some(1));
    let tenant_payload = json!({ "name": "Smoke Tenant" });
    let create_tenant_request = Request::builder()
        .method("POST")
        .uri("/tenants")
        .header("content-type", "application/json")
        .header("X-Tenant-ID", ROOT_TENANT_ID)
        .body(Body::from(tenant_payload.to_string()))?;
    let create_tenant_response = app.clone().oneshot(create_tenant_request).await?;
    assert_eq!(create_tenant_response.status(), StatusCode::OK);
    let create_tenant_bytes = create_tenant_response
        .into_body()
        .collect()
        .await?
        .to_bytes();
    let created_tenant: Value = serde_json::from_slice(&create_tenant_bytes)?;
    let tenant_id_str = created_tenant["id"]
        .as_str()
        .ok_or_else(|| anyhow!("missing tenant id"))?
        .to_string();
    let tenant_id = Uuid::parse_str(&tenant_id_str)?;
    assert_eq!(created_tenant["name"], json!("Smoke Tenant"));

    let list_tenants_request = Request::builder()
        .method("GET")
        .uri("/tenants")
        .header("X-Tenant-ID", ROOT_TENANT_ID)
        .body(Body::empty())?;
    let list_tenants_response = app.clone().oneshot(list_tenants_request).await?;
    assert_eq!(list_tenants_response.status(), StatusCode::OK);
    let list_tenants_bytes = list_tenants_response
        .into_body()
        .collect()
        .await?
        .to_bytes();
    let tenants_json: Value = serde_json::from_slice(&list_tenants_bytes)?;
    let tenants = tenants_json
        .as_array()
        .ok_or_else(|| anyhow!("tenants response not array"))?;
    assert!(tenants.iter().any(|entry| entry["id"] == tenant_id_str));

    let key_payload = json!({ "label": "Smoke Key" });
    let create_key_request = Request::builder()
        .method("POST")
        .uri(format!(
            "/tenants/{tenant}/integration-keys",
            tenant = tenant_id
        ))
        .header("content-type", "application/json")
        .header("X-Tenant-ID", tenant_id_str.as_str())
        .body(Body::from(key_payload.to_string()))?;
    let create_key_response = app.clone().oneshot(create_key_request).await?;
    assert_eq!(create_key_response.status(), StatusCode::OK);
    let create_key_bytes = create_key_response.into_body().collect().await?.to_bytes();
    let created_key: Value = serde_json::from_slice(&create_key_bytes)?;
    let key_id_str = created_key["id"]
        .as_str()
        .ok_or_else(|| anyhow!("missing key id"))?
        .to_string();
    let key_id = Uuid::parse_str(&key_id_str)?;
    let api_key = created_key["api_key"]
        .as_str()
        .ok_or_else(|| anyhow!("missing api key"))?;
    assert!(!api_key.is_empty());

    let list_keys_request = Request::builder()
        .method("GET")
        .uri(format!(
            "/tenants/{tenant}/integration-keys",
            tenant = tenant_id
        ))
        .header("X-Tenant-ID", tenant_id_str.as_str())
        .body(Body::empty())?;
    let list_keys_response = app.clone().oneshot(list_keys_request).await?;
    assert_eq!(list_keys_response.status(), StatusCode::OK);
    let list_keys_bytes = list_keys_response.into_body().collect().await?.to_bytes();
    let keys_json: Value = serde_json::from_slice(&list_keys_bytes)?;
    let keys = keys_json
        .as_array()
        .ok_or_else(|| anyhow!("keys response not array"))?;
    assert!(keys.iter().any(|entry| entry["id"] == key_id_str));

    let revoke_request = Request::builder()
        .method("POST")
        .uri(format!("/integration-keys/{key}/revoke", key = key_id))
        .header("X-Tenant-ID", tenant_id_str.as_str())
        .body(Body::empty())?;
    let revoke_response = app.clone().oneshot(revoke_request).await?;
    assert_eq!(revoke_response.status(), StatusCode::OK);
    let revoke_bytes = revoke_response.into_body().collect().await?.to_bytes();
    let revoked_key: Value = serde_json::from_slice(&revoke_bytes)?;
    assert!(revoked_key["revoked_at"].is_string());

    let session_request = Request::builder()
        .method("GET")
        .uri("/session")
        .header(COOKIE, &cookie_pair)
        .body(Body::empty())?;
    let session_response = app.clone().oneshot(session_request).await?;
    assert_eq!(session_response.status(), StatusCode::OK);

    let logout_request = Request::builder()
        .method("POST")
        .uri("/logout")
        .header(COOKIE, &cookie_pair)
        .body(Body::empty())?;
    let logout_response = app.clone().oneshot(logout_request).await?;
    assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);
    let logout_cookie = logout_response
        .headers()
        .get(SET_COOKIE)
        .ok_or_else(|| anyhow!("missing logout cookie"))?
        .to_str()?;
    assert!(logout_cookie.contains("Max-Age=0"));

    let refresh_after_logout = Request::builder()
        .method("GET")
        .uri("/session")
        .header(COOKIE, &cookie_pair)
        .body(Body::empty())?;
    let refresh_response = app.clone().oneshot(refresh_after_logout).await?;
    assert_eq!(refresh_response.status(), StatusCode::UNAUTHORIZED);

    let recorded_events = kafka_recorder.drain();
    assert!(recorded_events.iter().all(|event| !event.key.is_empty()));
    let mut primary_actions = Vec::new();
    let mut dlq_actions = Vec::new();
    for event in &recorded_events {
        let value: Value = serde_json::from_str(&event.payload)?;
        let action = value["action"].as_str().unwrap_or("").to_string();
        if event.topic == "security.test.mfa.dlq" {
            dlq_actions.push(action);
        } else {
            primary_actions.push(action);
        }
    }
    assert!(primary_actions
        .iter()
        .any(|action| action == "mfa.challenge.failed"));
    assert!(primary_actions
        .iter()
        .any(|action| action == "mfa.challenge.missing_code"));
    assert!(primary_actions
        .iter()
        .any(|action| action == "mfa.enrollment.start"));
    assert!(primary_actions
        .iter()
        .any(|action| action == "mfa.enrollment.completed"));
    assert!(dlq_actions
        .iter()
        .any(|action| action == "mfa.challenge.missing_code"));

    sqlx::query("DELETE FROM auth_refresh_tokens WHERE user_id = $1")
        .bind(mfa_user.user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(mfa_user.user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(mfa_user.tenant_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM integration_keys WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM auth_refresh_tokens WHERE user_id = $1")
        .bind(enroll_user.user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(enroll_user.user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(enroll_user.tenant_id)
        .execute(&pool)
        .await?;

    sqlx::query("DELETE FROM auth_refresh_tokens WHERE user_id = $1")
        .bind(seeded.user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(seeded.user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(seeded.tenant_id)
        .execute(&pool)
        .await?;

    db.teardown().await?;
    Ok(())
}
