use crate::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use common_auth::{
    ensure_role, tenant_id_from_request, AuthContext, ROLE_ADMIN, ROLE_MANAGER, ROLE_SUPER_ADMIN,
};

use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

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

#[derive(Serialize)]
pub struct ForecastResult {
    next_day_sales: f64,
}

const ANALYTICS_VIEW_ROLES: &[&str] = &[ROLE_SUPER_ADMIN, ROLE_ADMIN, ROLE_MANAGER];

pub async fn get_summary(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Summary>, (StatusCode, String)> {
    ensure_role(&auth, ANALYTICS_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

    let rec = sqlx::query(
        "SELECT order_count, total_sales FROM daily_sales \
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
    })?;

    let (order_count, total_sales) = match rec {
        Some(row) => {
            let count: i64 = row.try_get("order_count").map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("DB row decode failed: {}", e),
                )
            })?;
            let total: Option<f64> = row.try_get("total_sales").map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("DB row decode failed: {}", e),
                )
            })?;
            (count as u64, total.unwrap_or(0.0))
        }
        None => (0, 0.0),
    };

    let mut top_items: Vec<TopItem> = Vec::new();
    if let Some(counts) = state.product_counts.lock().unwrap().get(&tenant_id) {
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

pub async fn get_forecast(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<ForecastResult>, (StatusCode, String)> {
    ensure_role(&auth, ANALYTICS_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    ensure_role(&auth, ANALYTICS_VIEW_ROLES)?;
    let tenant_id = tenant_id_from_request(&headers, &auth)?;

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

    Ok(Json(anomalies))
}
