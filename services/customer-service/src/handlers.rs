use crate::*; // bring in main module symbols when included from lib/main context
use axum::{extract::{State, Path, Query}, Json};
use uuid::Uuid;
use common_security::{SecurityCtxExtractor, roles::ensure_any_role};
use common_http_errors::{ApiError, ApiResult};

pub async fn create_customer(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Json(new_cust): Json<NewCustomer>,
) -> ApiResult<Json<Customer>> { crate::create_customer_impl(state, sec, new_cust).await }

pub async fn get_customer(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(customer_id): Path<Uuid>,
) -> ApiResult<Json<Customer>> { crate::get_customer_impl(state, sec, customer_id).await }

pub async fn update_customer(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Path(customer_id): Path<Uuid>,
    Json(payload): Json<UpdateCustomerRequest>,
) -> ApiResult<Json<Customer>> { crate::update_customer_impl(state, sec, customer_id, payload).await }

pub async fn search_customers(
    State(state): State<AppState>,
    SecurityCtxExtractor(sec): SecurityCtxExtractor,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<Customer>>> { crate::search_customers_impl(state, sec, params).await }
