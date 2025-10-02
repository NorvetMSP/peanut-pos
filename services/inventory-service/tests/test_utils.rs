use sqlx::{PgPool, Row};
use uuid::Uuid;
use reqwest::Client;
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

/// Seed tenant + product + default location + legacy inventory & inventory_items row.
/// Returns (tenant_id, product_id, location_id).
pub async fn seed_inventory_basics(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
    let tenant_id = Uuid::new_v4();
    let product_id = Uuid::new_v4();

    sqlx::query("INSERT INTO inventory (product_id, tenant_id, quantity, threshold) VALUES ($1,$2,$3,$4)")
        .bind(product_id)
        .bind(tenant_id)
        .bind(10)
        .bind(5)
        .execute(pool)
        .await
        .expect("seed inventory");

    sqlx::query("INSERT INTO locations (tenant_id, code, name, timezone) VALUES ($1,'DEFAULT','Default','UTC') ON CONFLICT DO NOTHING")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("seed location");

    let loc = sqlx::query("SELECT id FROM locations WHERE tenant_id = $1 AND code='DEFAULT'")
        .bind(tenant_id)
        .fetch_one(pool)
        .await
        .expect("fetch default location")
        .get::<Uuid,_>("id");

    sqlx::query("INSERT INTO inventory_items (tenant_id, product_id, location_id, quantity, threshold) VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
        .bind(tenant_id)
        .bind(product_id)
        .bind(loc)
        .bind(10)
        .bind(5)
        .execute(pool)
        .await
        .expect("seed inventory_items");

    (tenant_id, product_id, loc)
}

/// Ensure minimal tables exist for inventory tests when migrations are not executed.
pub async fn ensure_inventory_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("CREATE TABLE IF NOT EXISTS inventory (product_id uuid, tenant_id uuid, quantity int, threshold int)")
        .execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS inventory_items (product_id uuid, tenant_id uuid, location_id uuid, quantity int, threshold int)")
        .execute(pool).await?;
    Ok(())
}

/// Issue a dev JWT using the repo's dev private key.
pub fn issue_dev_jwt(tenant_id: Uuid, roles: &[&str], issuer: &str, audience: &str) -> String {
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
    let private_pem = std::fs::read_to_string("jwt-dev.pem").expect("read dev private key");
    let subject = Uuid::new_v4();
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: &subject.to_string(),
        tid: &tenant_id.to_string(),
        roles: roles.iter().map(|r| (*r).to_string()).collect(),
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

/// Create a reservation over HTTP, returning the order_id used.
pub async fn create_reservation_http(
    client: &Client,
    base_url: &str,
    tenant_id: Uuid,
    product_id: Uuid,
    location_id: Uuid,
) -> Uuid {
    let order_id = Uuid::new_v4();
    let token = issue_dev_jwt(tenant_id, &["admin"], "itest-issuer", "itest-aud");
    let body = serde_json::json!({
        "order_id": order_id,
        "items": [ { "product_id": product_id, "quantity": 3, "location_id": location_id } ]
    });
    let resp = client
        .post(format!("{base_url}/inventory/reservations"))
        .header("authorization", format!("Bearer {}", token))
        .header("x-tenant-id", tenant_id.to_string())
        .json(&body)
        .send()
        .await
        .expect("send reservation request");
    assert!(resp.status().is_success(), "reservation creation failed: {:?}", resp.text().await.ok());
    order_id
}
