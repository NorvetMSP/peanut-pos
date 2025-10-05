#![cfg(feature = "integration-tests")]

use axum::{Router, body::{Body, to_bytes}};
use http::{Request, StatusCode};
use order_service::{build_router, AppState, build_jwt_verifier_from_env};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

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
          idempotency_key text NULL,
          exchange_of_order_id uuid NULL
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
    "#).execute(pool).await;
}

async fn start_test_db() -> Option<sqlx::PgPool> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) => v,
        Err(_) => { eprintln!("SKIP exchange tests: TEST_DATABASE_URL not set"); return None; }
    };
    match sqlx::PgPool::connect(&url).await {
        Ok(pool) => { run_migrations(&pool).await; Some(pool) },
        Err(err) => { eprintln!("SKIP exchange tests: cannot connect: {err}"); None }
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

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("local-dev".to_string());
    let token = encode(&header, &claims, &encoding).expect("sign");
    (public_pem, token)
}

fn dec(cents: i64) -> bigdecimal::BigDecimal { use bigdecimal::BigDecimal; BigDecimal::from(cents) / BigDecimal::from(100i64) }

#[tokio::test]
async fn exchange_net_collect() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["manager", "cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Seed products: A=$10.00, B=$15.00
    let a = Uuid::new_v4();
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(a).bind(tenant).bind("A").bind(dec(1000)).bind("SKU-A").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert A");
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("B").bind(dec(1500)).bind("SKU-B").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert B");

    // Create original order with A (cash exact)
    let order_body = json!({
        "items": [{"sku": "SKU-A", "quantity": 1}],
        "payment_method": "cash",
        "payment": {"method": "cash", "amount_cents": 1000}
    });
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/orders/sku")
            .header("Content-Type","application/json")
            .header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles", "cashier")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(order_body.to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let original_order: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    let original_order_id = original_order["id"].as_str().unwrap().parse::<Uuid>().unwrap();

    // Exchange: return A, buy B with card amount equal to B total (1500)
    let exch_body = json!({
        "return_items": [{"product_id": a, "qty": 1}],
        "new_items": [{"sku": "SKU-B", "qty": 1}],
        "payment": {"method": "card", "amount_cents": 1500}
    });
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri(format!("/orders/{}/exchange", original_order_id))
            .header("Content-Type","application/json")
            .header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles", "manager")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(exch_body.to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(body["original_order_id"].as_str().unwrap(), &original_order_id.to_string());
    assert_eq!(body["refunded_cents"].as_i64().unwrap(), 1000);
    assert_eq!(body["new_order_total_cents"].as_i64().unwrap(), 1500);
    assert_eq!(body["net_delta_cents"].as_i64().unwrap(), 500);
    assert_eq!(body["net_direction"].as_str().unwrap(), "collect");

    // Verify linkage exists
    let exch_id = body["exchange_order_id"].as_str().unwrap().parse::<Uuid>().unwrap();
    let link: Option<Uuid> = sqlx::query_scalar("SELECT exchange_of_order_id FROM orders WHERE id = $1")
        .bind(exch_id).fetch_one(&pool).await.ok().flatten();
    assert_eq!(link, Some(original_order_id));
}

#[tokio::test]
async fn exchange_net_refund() {
    let Some(pool) = start_test_db().await else { return; };
    let tenant = Uuid::new_v4();
    let (pub_pem, token) = generate_key_and_token("https://auth.novapos.local", "novapos-admin", tenant, &["manager", "cashier"]);
    std::env::set_var("JWT_DEV_PUBLIC_KEY_PEM", pub_pem);
    let app = build_test_app(pool.clone()).await;

    // Seed products: A=$15.00, B=$10.00
    let a = Uuid::new_v4();
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(a).bind(tenant).bind("A").bind(dec(1500)).bind("SKU-A").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert A");
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(tenant).bind("B").bind(dec(1000)).bind("SKU-B").bind(Some("EXEMPT")).bind(true)
        .execute(&pool).await.expect("insert B");

    // Create original order with A (card exact)
    let order_body = json!({
        "items": [{"sku": "SKU-A", "quantity": 1}],
        "payment_method": "card",
        "payment": {"method": "card", "amount_cents": 1500}
    });
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/orders/sku")
            .header("Content-Type","application/json")
            .header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles", "cashier")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(order_body.to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let original_order: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    let original_order_id = original_order["id"].as_str().unwrap().parse::<Uuid>().unwrap();

    // Exchange: return A, buy B with card amount equal to B total (1000)
    let exch_body = json!({
        "return_items": [{"product_id": a, "qty": 1}],
        "new_items": [{"sku": "SKU-B", "qty": 1}],
        "payment": {"method": "card", "amount_cents": 1000}
    });
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri(format!("/orders/{}/exchange", original_order_id))
            .header("Content-Type","application/json")
            .header("X-Tenant-ID", tenant.to_string())
            .header("X-Roles", "manager")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::from(exch_body.to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&to_bytes(resp.into_body(), 1024*1024).await.unwrap()).unwrap();
    assert_eq!(body["refunded_cents"].as_i64().unwrap(), 1500);
    assert_eq!(body["new_order_total_cents"].as_i64().unwrap(), 1000);
    assert_eq!(body["net_delta_cents"].as_i64().unwrap(), -500);
    assert_eq!(body["net_direction"].as_str().unwrap(), "refund");
}
