//! OpenClaw-specific preflight endpoint wrapper.
//!
//! OpenClaw keeps a dedicated module for route clarity and parity with other
//! runner-specific wrappers.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

use crate::helpers::ui::payloads::OpenClawToolPreflightPayload;
use crate::helpers::ui::runner_tool_preflight_common::run_tool_preflight_for_runner;
use crate::runners::RunnerKind;
use crate::ui::WebState;

/// Backward-compatible OpenClaw preflight endpoint.
pub async fn api_openclaw_tool_preflight(
    State(state): State<WebState>,
    Json(payload): Json<OpenClawToolPreflightPayload>,
) -> impl IntoResponse {
    run_tool_preflight_for_runner(state, payload, RunnerKind::Openclaw.id()).await
}
