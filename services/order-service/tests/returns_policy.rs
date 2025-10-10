#![cfg(feature = "integration-tests")]

use axum::{Router, body::{Body, to_bytes}};
use http::{Request, StatusCode};
use order_service::{build_router, AppState, build_jwt_verifier_from_env};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

async fn run_migrations(pool: &sqlx::PgPool) {
    let _ = sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS orders (
          id uuid PRIMARY KEY,
          tenant_id uuid NOT NULL,
          total numeric NOT NULL,
          status text NOT NULL,
          customer_id uuid NULL,
          customer_name text NULL,
          customer_email text NULL,
          store_id uuid NULL,
          created_at timestamptz NOT NULL DEFAULT now(),
          offline boolean NOT NULL DEFAULT false,
          payment_method text NOT NULL,
          idempotency_key text NULL
        );
        CREATE TABLE IF NOT EXISTS order_items (
          id uuid PRIMARY KEY,
          order_id uuid NOT NULL,
          product_id uuid NOT NULL,
          quantity int NOT NULL,
          returned_quantity int NOT NULL DEFAULT 0,
          unit_price numeric NOT NULL,
          line_total numeric NOT NULL,
          created_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE TABLE IF NOT EXISTS order_returns (
          id uuid PRIMARY KEY,
          order_id uuid NOT NULL,
          tenant_id uuid NOT NULL,
          total numeric NOT NULL,
          reason text NULL,
          created_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE TABLE IF NOT EXISTS order_return_items (
          id uuid PRIMARY KEY,
          return_id uuid NOT NULL,
          order_item_id uuid NOT NULL,
          quantity int NOT NULL,
          line_total numeric NOT NULL,
          created_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE TABLE IF NOT EXISTS return_policies (
          tenant_id UUID NOT NULL,
          location_id UUID NULL,
          allow_window_days INTEGER NOT NULL DEFAULT 30,
          restock_fee_bps INTEGER NOT NULL DEFAULT 0,
          receipt_required BOOLEAN NOT NULL DEFAULT TRUE,
          manager_override_allowed BOOLEAN NOT NULL DEFAULT TRUE,
          updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
          PRIMARY KEY (tenant_id, location_id)
        );
        CREATE TABLE IF NOT EXISTS return_overrides (
          id UUID PRIMARY KEY,
          tenant_id UUID NOT NULL,
          order_id UUID NOT NULL,
          reason TEXT NOT NULL,
          issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
          used_at TIMESTAMPTZ NULL
        );
    "#).execute(pool).await;
}

async fn start_test_db() -> Option<sqlx::PgPool> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) => v,
        Err(_) => { eprintln!("SKIP returns_policy: TEST_DATABASE_URL not set"); return None; }
    };
    match sqlx::PgPool::connect(&url).await {
        Ok(pool) => { run_migrations(&pool).await; Some(pool) },
        Err(err) => { eprintln!("SKIP returns_policy: cannot connect: {err}"); None }
    }
}

async fn build_test_app(pool: sqlx::PgPool) -> Router {
    std::env::set_var("ORDER_BYPASS_INVENTORY", "1");
    std::env::set_var("JWT_ISSUER", "https://auth.novapos.local");
    std::env::set_var("JWT_AUDIENCE", "novapos-admin");
    let verifier = build_jwt_verifier_from_env().await.expect("jwt verifier");
    let state = AppState {
        db: pool,
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
        kafka_producer: panic!("kafka disabled in tests"),
        jwt_verifier: verifier,
        http_client: reqwest::Client::new(),
        inventory_base_url: "http://localhost:8087".to_string(),
        #[cfg(any(feature = "kafka", feature = "kafka-producer"))]
        audit_producer: None,
    };
    build_router(state)
}

fn generate_key_and_token(issuer: &str, audience: &str, tenant: Uuid, roles: &[&str]) -> (String, String) {
    use chrono::Utc;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding};
    use rsa::rand_core::OsRng;
    use rsa::RsaPrivateKey;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Claims<'a> {
        sub: String,
        tid: String,
        roles: Vec<String>,
        iss: &'a str,
        aud: &'a str,
        exp: i64,
        iat: i64,
    }

    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("keygen");
    let public_key = private_key.to_public_key();
    let private_pem = private_key.to_pkcs1_pem(LineEnding::LF).expect("pem");
    let public_pem = public_key.to_pkcs1_pem(LineEnding::LF).expect("pub pem");
    let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("encoding");

    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: Uuid::new_v4().to_string(),
        tid: tenant.to_string(),
        roles: roles.iter().map(|r| r.to_string()).collect(),
        iss: issuer,
        aud: audience,
        exp: now + 600,
        iat: now,
    };

    let token = encode(&Header::new(Algorithm::RS256), &claims, &encoding).expect("encode");
    (public_pem, token)
}

#[tokio::test]
async fn rejects_refund_outside_window() {
    let Some(pool) = start_test_db().await else { return; };
    let app = build_test_app(pool.clone()).await;

    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["manager"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);

    // Seed order 40 days ago
    let order_id = Uuid::new_v4();
    let product_id = Uuid::new_v4();
    sqlx::query("INSERT INTO orders (id, tenant_id, total, status, created_at, payment_method) VALUES ($1,$2,100,'COMPLETED', NOW() - INTERVAL '40 days', 'cash')")
        .bind(order_id).bind(tenant).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO order_items (id, order_id, product_id, quantity, unit_price, line_total) VALUES ($1,$2,$3,1,100,100)")
        .bind(Uuid::new_v4()).bind(order_id).bind(product_id).execute(&pool).await.unwrap();

    // Default policy is 30 days; expect window expired
    let body = json!({
        "order_id": order_id,
        "items": [{"product_id": product_id, "quantity": 1}]
    }).to_string();
    let res = app.clone().oneshot(Request::builder()
        .method("POST").uri("/orders/refund")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Tenant-ID", tenant.to_string())
        .header("content-type", "application/json")
        .body(Body::from(body)).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(res.into_body(), 1024 * 64).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("return_window_expired"));
}

#[tokio::test]
async fn applies_restock_fee_bps() {
    let Some(pool) = start_test_db().await else { return; };
    let app = build_test_app(pool.clone()).await;

    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["manager"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);

    // Upsert policy: 10% restock fee
    let policy_body = json!({
        "allow_window_days": 30,
        "restock_fee_bps": 1000,
        "receipt_required": true,
        "manager_override_allowed": true
    }).to_string();
    let res = app.clone().oneshot(Request::builder()
        .method("POST").uri("/admin/return_policies")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Tenant-ID", tenant.to_string())
        .header("content-type", "application/json")
        .body(Body::from(policy_body)).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Seed order 1 day ago, total 100, single item 100
    let order_id = Uuid::new_v4();
    let product_id = Uuid::new_v4();
    sqlx::query("INSERT INTO orders (id, tenant_id, total, status, created_at, payment_method) VALUES ($1,$2,100,'COMPLETED', NOW() - INTERVAL '1 days', 'cash')")
        .bind(order_id).bind(tenant).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO order_items (id, order_id, product_id, quantity, unit_price, line_total) VALUES ($1,$2,$3,1,100,100)")
        .bind(Uuid::new_v4()).bind(order_id).bind(product_id).execute(&pool).await.unwrap();

    let body = json!({
        "order_id": order_id,
        "items": [{"product_id": product_id, "quantity": 1}]
    }).to_string();
    let res = app.clone().oneshot(Request::builder()
        .method("POST").uri("/orders/refund")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Tenant-ID", tenant.to_string())
        .header("content-type", "application/json")
        .body(Body::from(body)).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 1024 * 64).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("PARTIAL_REFUNDED") || text.contains("REFUNDED"));

    // Verify the recorded return total reflects 10% restock fee (100 -> 90)
    #[derive(sqlx::FromRow)]
    struct ReturnRow { total: sqlx::types::BigDecimal }
    let row: ReturnRow = sqlx::query_as(
        "SELECT total FROM order_returns WHERE order_id = $1 ORDER BY created_at DESC LIMIT 1"
    )
    .bind(order_id)
    .fetch_one(&pool).await.unwrap();
    let expected = sqlx::types::BigDecimal::from(90);
    assert_eq!(row.total, expected);
}

#[tokio::test]
async fn override_token_bypasses_window_once() {
    let Some(pool) = start_test_db().await else { return; };
    let app = build_test_app(pool.clone()).await;

    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["manager"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);

    // Seed old order 60 days ago
    let order_id = Uuid::new_v4();
    let product_id = Uuid::new_v4();
    sqlx::query("INSERT INTO orders (id, tenant_id, total, status, created_at, payment_method) VALUES ($1,$2,100,'COMPLETED', NOW() - INTERVAL '60 days', 'cash')")
        .bind(order_id).bind(tenant).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO order_items (id, order_id, product_id, quantity, unit_price, line_total) VALUES ($1,$2,$3,1,100,100)")
        .bind(Uuid::new_v4()).bind(order_id).bind(product_id).execute(&pool).await.unwrap();

    // Issue override token
    let body = json!({"order_id": order_id, "reason": "policy exception"}).to_string();
    let res = app.clone().oneshot(Request::builder()
        .method("POST").uri("/admin/overrides/returns")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Tenant-ID", tenant.to_string())
        .header("content-type", "application/json")
        .body(Body::from(body)).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 1024 * 64).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token_id = v.get("override_token").unwrap().as_str().unwrap().to_string();

    // Use override token to refund
    let refund_body = json!({
        "order_id": order_id,
        "items": [{"product_id": product_id, "quantity": 1}]
    }).to_string();
    let res = app.clone().oneshot(Request::builder()
        .method("POST").uri("/orders/refund")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Tenant-ID", tenant.to_string())
        .header("X-Return-Override", token_id.clone())
        .header("content-type", "application/json")
        .body(Body::from(refund_body)).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Second attempt should fail (token consumed)
    let refund_body2 = json!({
        "order_id": order_id,
        "items": [{"product_id": product_id, "quantity": 1}]
    }).to_string();
    let res2 = app.clone().oneshot(Request::builder()
        .method("POST").uri("/orders/refund")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Tenant-ID", tenant.to_string())
        .header("X-Return-Override", token_id)
        .header("content-type", "application/json")
        .body(Body::from(refund_body2)).unwrap()).await.unwrap();
    assert_eq!(res2.status(), StatusCode::BAD_REQUEST);
}
