#![cfg(feature = "integration-tests")] // run with: cargo test -p order-service --features integration-tests --tests

use axum::http::HeaderMap;
use bigdecimal::BigDecimal;
use order_service::order_handlers::{ComputeOrderItemInput, ComputeOrderRequest};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

// Re-export the inner helper using a tiny shim in tests to avoid changing visibility.
use order_service::order_handlers::compute_with_db_inner as compute_with_db;

fn cents(n: i64) -> BigDecimal { BigDecimal::from(n) / BigDecimal::from(100i64) }

#[tokio::test]
async fn compute_uses_tax_code_std_and_exempt() {
    // Arrange: ephemeral database (assumes TEST_DATABASE_URL set or uses local default)
    let db_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string());
    let pool = PgPool::connect(&db_url).await.expect("connect db");

    // Use a unique tenant and a temp table namespace by appending a suffix if necessary.
    let tenant_id = Uuid::new_v4();

    // Ensure products table exists; if the real schema exists, this is a no-op insert-only test.
    // If running against a scratch DB, create a minimal products table shape.
    let _ = pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS products (
          id uuid PRIMARY KEY,
          tenant_id uuid NOT NULL,
          name text NOT NULL,
          price numeric NOT NULL,
          sku text,
          tax_code text,
          active boolean NOT NULL DEFAULT true
        );
        "#
    ).await;

    // Clean any prior rows for this tenant
    let _ = sqlx::query("DELETE FROM products WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await;

    // Insert two products: STD taxable and EXEMPT
    let p_std = Uuid::new_v4();
    let p_exempt = Uuid::new_v4();
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(p_std).bind(tenant_id).bind("Soda Can").bind(cents(199)).bind("SKU-SODA").bind(Some("STD".to_string())).bind(true)
        .execute(&pool).await.expect("insert std");
    sqlx::query("INSERT INTO products (id, tenant_id, name, price, sku, tax_code, active) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(p_exempt).bind(tenant_id).bind("Bottle Water").bind(cents(149)).bind("SKU-WATER").bind(Some("EXEMPT".to_string())).bind(true)
        .execute(&pool).await.expect("insert exempt");

    // Request: 2x soda (taxable), 1x water (exempt), 10% discount. Override tax via header 800 bps (8%).
    let req = ComputeOrderRequest {
        items: vec![
            ComputeOrderItemInput { sku: Some("SKU-SODA".into()), product_id: None, quantity: 2 },
            ComputeOrderItemInput { sku: Some("SKU-WATER".into()), product_id: None, quantity: 1 },
        ],
        discount_percent_bp: Some(1000),
        location_id: None,
        pos_instance_id: None,
        tax_rate_bps: None,
    };

    let mut headers = HeaderMap::new();
    headers.insert("X-Tax-Rate-Bps", "800".parse().unwrap());

    // Act
    let resp = compute_with_db(&pool, tenant_id, &headers, &req).await.expect("compute ok");

    // Assert math:
    // Prices: soda 1.99 x2 = 3.98; water 1.49 x1 = 1.49; subtotal = 5.47 -> 547 cents
    // Discount 10% of subtotal = 54.7 -> round half up => 55 cents
    // Proportional discount on taxable base (soda 398/547 of 55) ~ 40.0 -> 40 cents
    // Taxable net = 398 - 40 = 358; tax @8% = 28.64 -> 29 cents
    // Total = 547 - 55 + 29 = 521
    assert_eq!(resp.subtotal_cents, 547);
    assert_eq!(resp.discount_cents, 55);
    assert_eq!(resp.tax_cents, 29);
    assert_eq!(resp.total_cents, 521);

    // Item summaries should reflect names, unit prices and line subtotals
    assert_eq!(resp.items.len(), 2);
    let soda = resp.items.iter().find(|i| i.sku.as_deref() == Some("SKU-SODA")).unwrap();
    assert_eq!(soda.name.as_deref(), Some("Soda Can"));
    assert_eq!(soda.unit_price_cents, 199);
    assert_eq!(soda.line_subtotal_cents, 398);
}
