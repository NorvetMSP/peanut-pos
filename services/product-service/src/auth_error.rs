use axum::{http::StatusCode, Json};
use serde::Serialize;
use uuid::Uuid;
use common_security::roles::Role;

#[derive(Serialize)]
pub struct AuthErrorBody<'a> {
    pub code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<Uuid>,
}

pub fn forbidden_missing_role(role: Option<&Role>, trace_id: Option<Uuid>) -> (StatusCode, Json<AuthErrorBody<'static>>) {
    let role_str = role.map(|r| match r { Role::Admin => "Admin", Role::Manager => "Manager", Role::Support => "Support", Role::Cashier => "Cashier" });
    (StatusCode::FORBIDDEN, Json(AuthErrorBody { code: "missing_role", missing_role: role_str, trace_id }))
}

pub fn forbidden_generic(trace_id: Option<Uuid>) -> (StatusCode, Json<AuthErrorBody<'static>>) {
    (StatusCode::FORBIDDEN, Json(AuthErrorBody { code: "forbidden", missing_role: None, trace_id }))
}

pub fn bad_request(code: &'static str, trace_id: Option<Uuid>) -> (StatusCode, Json<AuthErrorBody<'static>>) {
    (StatusCode::BAD_REQUEST, Json(AuthErrorBody { code, missing_role: None, trace_id }))
}