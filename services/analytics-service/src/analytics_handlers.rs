#[derive(Serialize)]
pub struct TopItem {
    pub product_id: Uuid,
    pub quantity: i32,
}

#[derive(Serialize)]
pub struct Summary {
    pub today_orders: u64,
    pub today_revenue: f64,
    pub top_items: Vec<TopItem>,
}

pub async fn get_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let tenant_id = extract_tenant_id(&headers)?;
    // Query today's aggregate from DB (or use in-memory Stats)
    let rec = sqlx::query!(
        "SELECT order_count, total_sales FROM daily_sales \
         WHERE tenant_id = $1 AND date = CURRENT_DATE",
        tenant_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB query failed: {}", e),
        )
    })?;
    let (order_count, total_sales) = rec
        .map(|r| (r.order_count as u64, r.total_sales.unwrap_or(0.0)))
        .unwrap_or((0, 0.0));
    // Get top 5 products
    let mut top_items: Vec<TopItem> = Vec::new();
    if let Some(counts) = state.product_counts.lock().unwrap().get(&tenant_id) {
        // Collect and sort by quantity desc
        for (&pid, &qty) in counts.iter() {
            if qty > 0 {
                top_items.push(TopItem {
                    product_id: pid,
                    quantity: qty,
                });
            }
        }
        top_items.sort_by(|a, b| b.quantity.cmp(&a.quantity));
        if top_items.len() > 5 {
            top_items.truncate(5);
        }
    }
    Ok(Json(Summary {
        today_orders: order_count,
        today_revenue: total_sales,
        top_items,
    }))
}
use crate::AppState;
use axum::http::StatusCode;
use axum::{extract::State, http::HeaderMap, Json};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ForecastResult {
    next_day_sales: f64,
}

pub async fn get_forecast(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ForecastResult>, (StatusCode, String)> {
    // Extract tenant_id from header
    let tenant_id = extract_tenant_id(&headers)?;
    // Query last 7 days of sales for this tenant
    let rows = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT total_sales FROM daily_sales \
             WHERE tenant_id = $1 AND date < CURRENT_DATE \
             ORDER BY date DESC LIMIT 7",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB query failed: {}", e),
        )
    })?;
    let values: Vec<f64> = rows.into_iter().flatten().collect();
    if values.is_empty() {
        return Ok(Json(ForecastResult {
            next_day_sales: 0.0,
        }));
    }
    let sum: f64 = values.iter().sum();
    let avg = sum / (values.len() as f64);
    Ok(Json(ForecastResult {
        next_day_sales: avg,
    }))
}

pub async fn get_anomalies(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let tenant_id = extract_tenant_id(&headers)?;
    // Get average refund amount (last 7 days) and today's refunds
    let avg_refund = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(refund_amount) FROM daily_sales \
             WHERE tenant_id = $1 AND date < CURRENT_DATE",
    )
    .bind(tenant_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB query failed: {}", e),
        )
    })?
    .unwrap_or(0.0);
    let today_refund = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT refund_amount FROM daily_sales \
             WHERE tenant_id = $1 AND date = CURRENT_DATE",
    )
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DB query failed: {}", e),
        )
    })?
    .flatten()
    .unwrap_or(0.0);
    let mut anomalies = Vec::new();
    if avg_refund > 0.0 && today_refund > 2.0 * avg_refund {
        anomalies.push(format!(
            "High refund volume detected: ${:.2} refunded in last 24h vs ${:.2} avg",
            today_refund, avg_refund
        ));
    }
    // (Additional anomaly checks can be added here)
    Ok(Json(anomalies))
}

// Helper to extract tenant UUID from headers (similar to existing code)
fn extract_tenant_id(headers: &HeaderMap) -> Result<Uuid, (StatusCode, String)> {
    headers
        .get("X-Tenant-ID")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or invalid X-Tenant-ID".into(),
        ))
}
