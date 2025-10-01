//! Integration test for multi-location reservation lifecycle (create -> expire -> restock).
//! NOTE: Spins up ephemeral Postgres with testcontainers; requires Docker available.

use uuid::Uuid;
use reqwest::Client;
use std::{env, time::Duration};
use tokio::process::Command;
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use testcontainers::core::WaitFor;

#[tokio::test]
async fn reservation_expires_and_restocks() {
    // Skip in CI unless explicitly enabled
    if env::var("ENABLE_ITESTS").ok().as_deref() != Some("1") { return; }

    // Postgres
    let pg_image = GenericImage::new("postgres", "16-alpine")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_wait_for(WaitFor::message_on_stdout("database system is ready to accept connections"));
    let container: ContainerAsync<GenericImage> = pg_image.start().await;
    let host_port = container.get_host_port_ipv4(5432).await;
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");

    // Redpanda (Kafka-compatible) lightweight broker
    let kafka_image = GenericImage::new("docker.redpanda.com/redpanda/redpanda", "v23.3.10")
        .with_env_var("REDPANDA_ENABLE_SASL", "false")
        .with_env_var("REDPANDA_AUTO_CREATE_TOPICS", "true")
        .with_wait_for(WaitFor::message_on_stdout("Successfully started Redpanda"))
        .with_exposed_port(9092);
    let kafka: ContainerAsync<GenericImage> = kafka_image.start().await;
    // Redpanda maps 9092 inside container; resolve host port
    let kafka_port = kafka.get_host_port_ipv4(9092).await;
    let kafka_bootstrap = format!("127.0.0.1:{kafka_port}");

    env::set_var("DATABASE_URL", &db_url); // ensure service sees it
    env::set_var("MULTI_LOCATION_ENABLED", "true");
    env::set_var("RESERVATION_DEFAULT_TTL_SECS", "2");
    env::set_var("RESERVATION_EXPIRY_SWEEP_SECS", "1");
    env::set_var("KAFKA_BOOTSTRAP", &kafka_bootstrap);

    // Spawn service binary (it will create tables via migrations on startup if logic exists; else we run them here where needed).
    let mut child = Command::new("cargo")
        .args(["run", "-p", "inventory-service"])
        .env("PORT", "48087")
        .env("HOST", "127.0.0.1")
        .kill_on_drop(true)
        .spawn()
        .expect("launch inventory-service");

    // crude readiness wait
    // Poll health until ready or timeout
    let start = std::time::Instant::now();
    let client = Client::new();
    loop {
        if start.elapsed() > Duration::from_secs(20) { panic!("service did not become ready"); }
        if let Ok(r) = client.get("http://127.0.0.1:48087/healthz").send().await { if r.status().is_success() { break; } }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // Seed product & legacy inventory directly (simulate product.created)
    let tenant_id = Uuid::new_v4();
    let product_id = Uuid::new_v4();
    let pool = PgPool::connect(&db_url).await.unwrap();
    // Best-effort migrate (if already applied it will be idempotent)
    // Optional: if migrations embedded: sqlx::migrate!("../../inventory-service/migrations").run(&pool).await.ok();
    // Insert legacy inventory
    sqlx::query!("INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1,$2,$3,$4)", product_id, tenant_id, 10, 5).execute(&pool).await.unwrap();
    // Backfill default location (simulate migration 4005)
    sqlx::query!("INSERT INTO locations (tenant_id, code, name, timezone) VALUES ($1,'DEFAULT','Default','UTC') ON CONFLICT DO NOTHING", tenant_id).execute(&pool).await.unwrap();
    let loc = sqlx::query!("SELECT id FROM locations WHERE tenant_id = $1 AND code='DEFAULT'", tenant_id).fetch_one(&pool).await.unwrap().id;
    sqlx::query!("INSERT INTO inventory_items (tenant_id, product_id, location_id, quantity, threshold) VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING", tenant_id, product_id, loc, 10, 5).execute(&pool).await.unwrap();

    // Prepare authenticated reservation creation via HTTP using a dev-signed JWT
    let order_id = Uuid::new_v4();
    // Provide JWT config env vars so the service verifier accepts our token.
    env::set_var("JWT_ISSUER", "itest-issuer");
    env::set_var("JWT_AUDIENCE", "itest-aud");
    // Inject dev public key so verifier can use it
    let public_pem = std::fs::read_to_string("jwt-dev.pub.pem").expect("read dev public key");
    env::set_var("JWT_DEV_PUBLIC_KEY_PEM", public_pem);

    // Issue a JWT using private key material
    let private_pem = std::fs::read_to_string("jwt-dev.pem").expect("read dev private key");
    let token = issue_dev_jwt(&private_pem, tenant_id, &["admin"], "itest-issuer", "itest-aud");

    let reservation_body = serde_json::json!({
        "order_id": order_id,
        "items": [ { "product_id": product_id, "quantity": 3, "location_id": loc } ]
    });
    let resp = client
        .post("http://127.0.0.1:48087/inventory/reservations")
        .header("authorization", format!("Bearer {}", token))
        .header("x-tenant-id", tenant_id.to_string())
        .json(&reservation_body)
        .send()
        .await
        .expect("send reservation request");
    assert!(resp.status().is_success(), "reservation creation failed: {:?}", resp.text().await.ok());

    // Wait for sweeper (1s sweep + TTL 2s)
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Assert Kafka events (reservation.expired + audit.events) appeared
    // Simple consumer using rdkafka
    use rdkafka::{consumer::{StreamConsumer, Consumer}, ClientConfig, Message};
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &kafka_bootstrap)
        .set("group.id", &format!("itest-{}", Uuid::new_v4()))
        .set("enable.partition.eof", "false")
        .set("auto.offset.reset", "earliest")
        .create()
        .expect("create consumer");
    consumer.subscribe(&["inventory.reservation.expired", "audit.events"]).expect("subscribe");
    let mut saw_reservation = false;
    let mut saw_audit = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while (!saw_reservation || !saw_audit) && std::time::Instant::now() < deadline {
        if let Ok(result) = tokio::time::timeout(Duration::from_millis(500), consumer.recv()).await {
            if let Ok(msg) = result {
                let topic = msg.topic();
                let payload = msg.payload().and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("");
                if topic == "inventory.reservation.expired" && payload.contains(&product_id.to_string()) { saw_reservation = true; }
                if topic == "audit.events" && payload.contains("reservation.expired") { saw_audit = true; }
            }
        }
    }
    assert!(saw_reservation, "expected inventory.reservation.expired event");
    assert!(saw_audit, "expected audit.events reservation.expired audit event");

    // Assert reservation expired
    let active = sqlx::query!("SELECT count(*) as ct FROM inventory_reservations WHERE tenant_id=$1 AND product_id=$2 AND status='ACTIVE'", tenant_id, product_id).fetch_one(&pool).await.unwrap().ct.unwrap_or(0);
    assert_eq!(active, 0, "active reservations should be zero after expiry");
    // Assert inventory restored
    let qty = sqlx::query!("SELECT SUM(quantity) as q FROM inventory_items WHERE tenant_id=$1 AND product_id=$2", tenant_id, product_id).fetch_one(&pool).await.unwrap().q.unwrap_or(0);
    assert_eq!(qty, 10, "quantity should be restored to 10 after expiration");

    let _ = child.kill().await; // cleanup
}

fn issue_dev_jwt(private_pem: &str, tenant_id: Uuid, roles: &[&str], issuer: &str, audience: &str) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use chrono::Utc;
    #[derive(serde::Serialize)]
    struct Claims<'a> {
        sub: &'a str,
        #[serde(rename = "tid")] tid: &'a str,
        roles: Vec<String>,
        iss: &'a str,
        aud: &'a str,
        exp: i64,
        iat: i64,
    }
    let subject = Uuid::new_v4();
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: &subject.to_string(),
        tid: &tenant_id.to_string(),
        roles: roles.iter().map(|r| r.to_string()).collect(),
        iss: issuer,
        aud: audience,
        exp: now + 600,
        iat: now,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("local-dev".to_string());
    let key = EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("valid private key");
    encode(&header, &claims, &key).expect("jwt encode")
}
