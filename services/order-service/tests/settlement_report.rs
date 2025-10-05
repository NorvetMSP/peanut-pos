// Integration tests for the settlement (Z-report) endpoint.
// Run with:
//   cargo test -p order-service --no-default-features --features "integration-tests" --tests -- --test-threads=1

#![cfg(feature = "integration-tests")]

use axum::{Router, body::{Body, to_bytes}};
use http::{Request, StatusCode};
use order_service::{build_router, AppState, build_jwt_verifier_from_env};
use tower::ServiceExt;
use uuid::Uuid;

// --- Minimal migrations helper (duplicated from payments_integration.rs to avoid cross-test deps) ---
async fn run_migrations(pool: &sqlx::PgPool) {
    let _ = sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS products (
          id uuid PRIMARY KEY,
          tenant_id uuid NOT NULL,
          name text NOT NULL,
          price numeric NOT NULL,
          sku text,
          tax_code text,
          active boolean NOT NULL DEFAULT true
        );
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
          order_id uuid NOT NULL,
          product_id uuid NOT NULL,
          quantity int NOT NULL,
          unit_price numeric NOT NULL,
          line_total numeric NOT NULL
        );
        CREATE TABLE IF NOT EXISTS payments (
          id uuid PRIMARY KEY,
          tenant_id uuid NOT NULL,
          order_id uuid NOT NULL,
          method text NOT NULL,
          amount numeric NOT NULL,
          status text NOT NULL,
          change_cents int NULL,
          created_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE TABLE IF NOT EXISTS tax_rate_overrides (
          tenant_id uuid NOT NULL,
          location_id uuid NULL,
          pos_instance_id uuid NULL,
          rate_bps int NOT NULL,
          updated_at timestamptz NOT NULL DEFAULT now()
        );
    "#).execute(pool).await;
}

async fn start_test_db() -> Option<sqlx::PgPool> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP settlement tests: TEST_DATABASE_URL not set");
            return None;
        }
    };
    match sqlx::PgPool::connect(&url).await {
        Ok(pool) => { run_migrations(&pool).await; Some(pool) },
        Err(err) => { eprintln!("SKIP settlement tests: cannot connect to TEST_DATABASE_URL: {err}"); None }
    }
}

async fn build_test_app(pool: sqlx::PgPool) -> Router {
    // Configure verifier via env to use dev pem, and bypass inventory calls
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

// Generate an ephemeral RSA key pair and sign a JWT acceptable by the dev verifier
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

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("local-dev".to_string());
    let token = encode(&header, &claims, &encoding).expect("sign");
    (public_pem, token)
}

#[tokio::test]
async fn settlement_report_groups_by_method_and_sums_amounts() {
    let Some(pool) = start_test_db().await else { return; };

    // Prepare JWT and verifier key
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Fixed date for determinism
    let date = "2024-01-02"; // YYYY-MM-DD

    // Seed payments: 2 cash (1.50 + 2.00), 1 card (3.25), all 'captured' for the target date
    let order1 = Uuid::new_v4();
    let order2 = Uuid::new_v4();
    let order3 = Uuid::new_v4();
    let order_other_day = Uuid::new_v4();

    // Insert rows into orders to satisfy any potential FK (none in test schema, but harmless)
    let _ = sqlx::query("INSERT INTO orders (id, tenant_id, total, status, payment_method) VALUES ($1,$2,$3,$4,$5)")
        .bind(order1).bind(tenant).bind(dec(150)).bind("completed").bind("cash").execute(&pool).await;
    let _ = sqlx::query("INSERT INTO orders (id, tenant_id, total, status, payment_method) VALUES ($1,$2,$3,$4,$5)")
        .bind(order2).bind(tenant).bind(dec(200)).bind("completed").bind("cash").execute(&pool).await;
    let _ = sqlx::query("INSERT INTO orders (id, tenant_id, total, status, payment_method) VALUES ($1,$2,$3,$4,$5)")
        .bind(order3).bind(tenant).bind(dec(325)).bind("completed").bind("card").execute(&pool).await;
    let _ = sqlx::query("INSERT INTO orders (id, tenant_id, total, status, payment_method) VALUES ($1,$2,$3,$4,$5)")
        .bind(order_other_day).bind(tenant).bind(dec(999)).bind("completed").bind("cash").execute(&pool).await;

    // Insert payments with explicit created_at timestamps to control the date
    let ts = "2024-01-02 12:00:00+00"; // noon UTC
    sqlx::query("INSERT INTO payments (id, tenant_id, order_id, method, amount, status, created_at) VALUES ($1,$2,$3,$4,$5,$6, $7::timestamptz)")
        .bind(Uuid::new_v4()).bind(tenant).bind(order1).bind("cash").bind(dec(150)).bind("captured").bind(ts)
        .execute(&pool).await.expect("insert p1");
    sqlx::query("INSERT INTO payments (id, tenant_id, order_id, method, amount, status, created_at) VALUES ($1,$2,$3,$4,$5,$6, $7::timestamptz)")
        .bind(Uuid::new_v4()).bind(tenant).bind(order2).bind("cash").bind(dec(200)).bind("captured").bind(ts)
        .execute(&pool).await.expect("insert p2");
    sqlx::query("INSERT INTO payments (id, tenant_id, order_id, method, amount, status, created_at) VALUES ($1,$2,$3,$4,$5,$6, $7::timestamptz)")
        .bind(Uuid::new_v4()).bind(tenant).bind(order3).bind("card").bind(dec(325)).bind("captured").bind(ts)
        .execute(&pool).await.expect("insert p3");
    // This one should be excluded by date filter
    sqlx::query("INSERT INTO payments (id, tenant_id, order_id, method, amount, status, created_at) VALUES ($1,$2,$3,$4,$5,$6, $7::timestamptz)")
        .bind(Uuid::new_v4()).bind(tenant).bind(order_other_day).bind("cash").bind(dec(777)).bind("captured").bind("2024-01-01 12:00:00+00")
        .execute(&pool).await.expect("insert p4 other day");

    // Call report endpoint
    let resp = app.clone().oneshot(
        Request::builder()
            .method("GET")
            .uri(format!("/reports/settlement?date={}", date))
            .header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles", "admin")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["date"].as_str().unwrap(), date);
    let totals = body["totals"].as_array().unwrap();

    // Find by method to avoid relying on ordering
    let find = |m: &str| totals.iter().find(|v| v["method"].as_str() == Some(m)).cloned();
    let cash = find("cash").expect("cash present");
    let card = find("card").expect("card present");

    assert_eq!(cash["count"].as_i64().unwrap(), 2);
    assert_eq!(cash["amount"].as_str().unwrap_or(""), "3.50");
    assert_eq!(card["count"].as_i64().unwrap(), 1);
    assert_eq!(card["amount"].as_str().unwrap_or(""), "3.25");
}

#[tokio::test]
async fn settlement_report_requires_admin_like_roles() {
    let Some(pool) = start_test_db().await else { return; };

    // Prepare JWTs
    let tenant = Uuid::new_v4();
    let (pub_pem, token_cashier) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    let date = "2024-01-02";

    // Cashier-only role should be forbidden
    let resp = app.clone().oneshot(
        Request::builder()
            .method("GET")
            .uri(format!("/reports/settlement?date={}", date))
            .header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles", "cashier")
            .header("Authorization", format!("Bearer {}", token_cashier))
            .body(Body::empty())
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

fn dec(cents: i64) -> bigdecimal::BigDecimal {
    use bigdecimal::BigDecimal;
    BigDecimal::from(cents) / BigDecimal::from(100i64)
}
