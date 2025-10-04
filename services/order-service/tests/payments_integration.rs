// Feature-gated integration tests for payment flows.
// Run with:
//   cargo test -p order-service --no-default-features --features "integration-tests" --tests -- --test-threads=1

#![cfg(feature = "integration-tests")]

use axum::{Router, body::{Body, to_bytes}};
use http::{Request, StatusCode};
use order_service::{build_router, AppState, build_jwt_verifier_from_env};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

// --- Minimal migrations helper ---
async fn run_migrations(pool: &sqlx::PgPool) {
    // Reuse production migrations; sqlx CLI not available here, so run via embedded path
    // Create subset tables needed for tests if migrations path is inaccessible
    // Fallback: rely on existing migrations in repo; otherwise create minimal tables
    // Minimal schema for products, orders, order_items, payments, tax_rate_overrides
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
            eprintln!("SKIP payments tests: TEST_DATABASE_URL not set");
            return None;
        }
    };
    match sqlx::PgPool::connect(&url).await {
        Ok(pool) => { run_migrations(&pool).await; Some(pool) },
        Err(err) => { eprintln!("SKIP payments tests: cannot connect to TEST_DATABASE_URL: {err}"); None }
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
async fn cash_happy_path_and_change() {
    let Some(pool) = start_test_db().await else { return; };
    // Prepare JWT and verifier key
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin", "cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Seed products
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Soda Can").bind(dec(199)).bind("SKU-SODA").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert soda");
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Bottle Water").bind(dec(149)).bind("SKU-WATER").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert water");

    // Compute
    let compute_body = json!({
        "items": [
            {"sku": "SKU-SODA", "quantity": 2},
            {"sku": "SKU-WATER", "quantity": 1}
        ],
        "discount_percent_bp": 1000,
        "tax_rate_bps": 800
    });
    let resp = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/orders/compute")
            .header("Content-Type", "application/json")
            .header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles", "admin")
        .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(compute_body.to_string()))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let comp: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let total_cents = comp["total_cents"].as_i64().unwrap();

    // Create order with cash, paying more to get change
    let pay_cents = total_cents + 100; // +$1.00
    let order_body = json!({
        "items": [
            {"sku": "SKU-SODA", "quantity": 2},
            {"sku": "SKU-WATER", "quantity": 1}
        ],
        "discount_percent_bp": 1000,
        "tax_rate_bps": 800,
        "payment_method": "cash",
        "payment": {"method": "cash", "amount_cents": pay_cents}
    });
    let resp = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/orders/sku")
            .header("Content-Type", "application/json")
            .header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles", "cashier")
        .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(order_body.to_string()))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let order: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let order_id = order["id"].as_str().unwrap().parse::<Uuid>().unwrap();

    // Receipt should include Paid and Change
    let resp = app.clone().oneshot(
        Request::builder()
            .method("GET")
            .uri(format!("/orders/{}/receipt", order_id))
            .header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles", "admin")
        .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let txt = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(txt.contains("Paid:"));
    assert!(txt.contains("Change:"));
}

#[tokio::test]
async fn card_exact_amount_happy_path() {
    let Some(pool) = start_test_db().await else { return; };
    // Prepare JWT and verifier key
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin", "cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Seed a single product
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Protein Bar").bind(dec(299)).bind("SKU-BAR").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert bar");

    // Compute
    let compute_body = json!({"items": [{"sku": "SKU-BAR", "quantity": 1}], "tax_rate_bps": 800});
    let resp = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/orders/compute")
            .header("Content-Type", "application/json")
            .header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles", "admin")
        .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(compute_body.to_string()))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let comp: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let total_cents = comp["total_cents"].as_i64().unwrap();

    // Create order with card and exact amount
    let order_body = json!({
        "items": [{"sku": "SKU-BAR", "quantity": 1}],
        "tax_rate_bps": 800,
        "payment_method": "card",
        "payment": {"method": "card", "amount_cents": total_cents}
    });
    let resp = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/orders/sku")
            .header("Content-Type", "application/json")
            .header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles", "cashier")
        .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(order_body.to_string()))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let order: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let order_id = order["id"].as_str().unwrap().parse::<Uuid>().unwrap();

    // Receipt should include Paid and not include Change
    let resp = app.clone().oneshot(
        Request::builder()
            .method("GET")
            .uri(format!("/orders/{}/receipt", order_id))
            .header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles", "admin")
        .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let txt = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(txt.contains("Paid:"));
    assert!(!txt.contains("Change:"));
}

fn dec(cents: i64) -> bigdecimal::BigDecimal {
    use bigdecimal::BigDecimal;
    BigDecimal::from(cents) / BigDecimal::from(100i64)
}

// --- Additional MVP-negative and guard tests ---

#[tokio::test]
async fn cash_insufficient_funds_rejected() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Water").bind(dec(150)).bind("SKU-WATER").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert");

    // Compute total
    let compute_body = json!({"items": [{"sku":"SKU-WATER","quantity":1}], "tax_rate_bps": 0});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token))
        .body(Body::from(compute_body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let total = serde_json::from_slice::<serde_json::Value>(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap()["total_cents"].as_i64().unwrap();

    // Pay less than total -> 400 insufficient_cash
    let order_body = json!({
        "items": [{"sku":"SKU-WATER","quantity":1}],
        "payment_method": "cash",
        "payment": {"method":"cash","amount_cents": total - 1}
    });
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/sku")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token))
        .body(Body::from(order_body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn card_amount_mismatch_rejected() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Bar").bind(dec(200)).bind("SKU-BAR").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert");

    // Compute
    let compute_body = json!({"items": [{"sku": "SKU-BAR", "quantity": 1}], "tax_rate_bps": 0});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token))
        .body(Body::from(compute_body.to_string())).unwrap()).await.unwrap();
    let total = serde_json::from_slice::<serde_json::Value>(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap()["total_cents"].as_i64().unwrap();

    // Mismatch: amount != total
    let order_body = json!({"items":[{"sku":"SKU-BAR","quantity":1}], "payment_method":"card", "payment": {"method":"card","amount_cents": total + 1}});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/sku")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token))
        .body(Body::from(order_body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_payment_rejected_for_cash_and_card() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Candy").bind(dec(100)).bind("SKU-CANDY").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert");

    for method in ["cash", "card"] {
        let order_body = json!({"items":[{"sku":"SKU-CANDY","quantity":1}], "payment_method": method});
        let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/sku")
            .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token))
            .body(Body::from(order_body.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn idempotency_returns_same_order() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Gum").bind(dec(75)).bind("SKU-GUM").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert");

    let idempotency_key = format!("key-{}", Uuid::new_v4());
    let order_body = json!({
        "items": [{"sku":"SKU-GUM","quantity":1}],
        "payment_method":"cash",
        "payment": {"method":"cash","amount_cents": 75},
        "idempotency_key": idempotency_key
    });
    let req = || Request::builder().method("POST").uri("/orders/sku")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token))
        .body(Body::from(order_body.to_string())).unwrap();
    let resp1 = app.clone().oneshot(req()).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let o1: serde_json::Value = serde_json::from_slice(&to_bytes(resp1.into_body(), 1024*1024).await.unwrap()).unwrap();
    let id1 = o1["id"].as_str().unwrap().to_string();
    let resp2 = app.clone().oneshot(req()).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let o2: serde_json::Value = serde_json::from_slice(&to_bytes(resp2.into_body(), 1024*1024).await.unwrap()).unwrap();
    let id2 = o2["id"].as_str().unwrap().to_string();
    assert_eq!(id1, id2);
}

#[tokio::test]
async fn auth_and_role_guards_enforced() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token_admin) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin"]);
    let (_pub_pem2, token_cashier) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Missing Authorization
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .body(Body::from("{}"))
        .unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Admin-only endpoint rejects cashier
    let upsert = json!({"rate_bps": 500});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","cashier").header("Authorization", format!("Bearer {}", token_cashier))
        .body(Body::from(upsert.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Admin can upsert successfully
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(upsert.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_tax_override_affects_compute() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token_admin) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Seed product with taxable code
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Taxed Item").bind(dec(1000)).bind("SKU-TAXED").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert");

    // Compute with no override (default 0 bps) => tax 0
    let compute_body = json!({"items": [{"sku":"SKU-TAXED","quantity":1}]});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(compute_body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v0: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(v0["tax_cents"].as_i64().unwrap(), 0);

    // Upsert tenant-level override to 1000 bps (10%)
    let upsert = json!({"rate_bps": 1000});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(upsert.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Compute again -> tax should be 100 cents on $10.00
    let compute_body = json!({"items": [{"sku":"SKU-TAXED","quantity":1}]});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(compute_body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v1: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(v1["tax_cents"].as_i64().unwrap(), 100);
    assert_eq!(v1["total_cents"].as_i64().unwrap(), 1100);
}

#[tokio::test]
async fn tax_override_precedence_pos_over_location_over_tenant() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token_admin) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Seed one taxable product @ $10
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Taxed").bind(dec(1000)).bind("SKU-TAXED").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert");

    // Prepare identifiers
    let location_id = Uuid::new_v4();
    let pos_id = Uuid::new_v4();

    // Upsert tenant-level 5%
    let up_tenant = json!({"rate_bps": 500});
    let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(up_tenant.to_string())).unwrap()).await.unwrap();

    // Compute with no location/pos -> 5%
    let compute_body = json!({"items":[{"sku":"SKU-TAXED","quantity":1}]});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(compute_body.to_string())).unwrap()).await.unwrap();
    let v_tenant: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(v_tenant["tax_cents"].as_i64().unwrap(), 50);

    // Upsert location-level 7%
    let up_loc = json!({"location_id": location_id, "rate_bps": 700});
    let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(up_loc.to_string())).unwrap()).await.unwrap();

    // Compute with location only -> 7%
    let compute_loc = json!({"items":[{"sku":"SKU-TAXED","quantity":1}], "location_id": location_id});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(compute_loc.to_string())).unwrap()).await.unwrap();
    let v_loc: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(v_loc["tax_cents"].as_i64().unwrap(), 70);

    // Upsert pos-level 9%
    let up_pos = json!({"location_id": location_id, "pos_instance_id": pos_id, "rate_bps": 900});
    let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(up_pos.to_string())).unwrap()).await.unwrap();

    // Compute with both location and pos -> 9%
    let compute_pos = json!({"items":[{"sku":"SKU-TAXED","quantity":1}], "location_id": location_id, "pos_instance_id": pos_id});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(compute_pos.to_string())).unwrap()).await.unwrap();
    let v_pos: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(v_pos["tax_cents"].as_i64().unwrap(), 90);
}

#[tokio::test]
async fn location_override_applies_when_pos_absent() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token_admin) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["admin"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("Taxed").bind(dec(1000)).bind("SKU-TAXED").bind(Some("STD")).bind(true)
        .execute(&pool).await.expect("insert");
    let location_id = Uuid::new_v4();

    // Tenant 5%, Location 7%
    let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(json!({"rate_bps": 500}).to_string())).unwrap()).await.unwrap();
    let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/tax_rate_overrides")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(json!({"location_id": location_id, "rate_bps": 700}).to_string())).unwrap()).await.unwrap();

    // Compute with location only -> 7%
    let compute_loc = json!({"items":[{"sku":"SKU-TAXED","quantity":1}], "location_id": location_id});
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/orders/compute")
        .header("Content-Type","application/json").header("X-Tenant-ID", tenant.to_string())
        .header("X-Roles","admin").header("Authorization", format!("Bearer {}", token_admin))
        .body(Body::from(compute_loc.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(v["tax_cents"].as_i64().unwrap(), 70);
}
