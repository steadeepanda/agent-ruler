//! OpenCode-specific preflight endpoint wrapper.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

use crate::helpers::ui::payloads::OpenClawToolPreflightPayload;
use crate::helpers::ui::runner_tool_preflight_common::run_tool_preflight_for_runner;
use crate::runners::RunnerKind;
use crate::ui::WebState;

pub async fn api_opencode_tool_preflight(
    State(state): State<WebState>,
    Json(payload): Json<OpenClawToolPreflightPayload>,
) -> impl IntoResponse {
    run_tool_preflight_for_runner(state, payload, RunnerKind::Opencode.id()).await
}
