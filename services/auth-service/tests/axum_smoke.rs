mod support;

use anyhow::{anyhow, Context, Result};
use auth_service::metrics::AuthMetrics;
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
    Request, StatusCode,
};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use common_auth::{JwtConfig, JwtVerifier};
use http_body_util::BodyExt;
use rand_core::OsRng;
use rdkafka::producer::FutureProducer;
use rdkafka::ClientConfig;
use reqwest::Client;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use serde::Serialize;
use serde_json::{json, Value};
use std::{str, sync::Arc};
use support::{default_auth_config, seed_test_user, TestDatabase};
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

    let kafka_producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", "localhost:9092")
        .create()
        .context("Failed to create Kafka producer")?;

    let http_client = Client::builder().build()?;

    let config = default_auth_config();
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
