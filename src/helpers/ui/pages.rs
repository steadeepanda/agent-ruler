//! Static UI asset responders extracted from `src/ui.rs`.
//!
//! This helper keeps UI route wiring in `src/ui.rs` readable while preserving a
//! deterministic include order for the WebUI bundle.

use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Redirect, Response};

const UI_SHELL_HTML: &str = include_str!("../../../assets/control-panel-shell.html");
const DESIGN_TOKENS_CSS: &str = include_str!("../../../assets/design-tokens.css");
const UI_CSS: &str = include_str!("../../../assets/ui.css");
const LOGO_SVG: &str = include_str!("../../../assets/logo-mark.svg");
// Keep concatenation order explicit: base/runtime helpers -> page modules ->
// shared shell/notification/bootstrap glue.
const UI_JS: &str = concat!(
    include_str!("../../../assets/ui/00-core.js"),
    include_str!("../../../assets/ui/path-browser-utils.js"),
    include_str!("../../../assets/ui/pages/main/overview.js"),
    include_str!("../../../assets/ui/pages/main/approvals.js"),
    include_str!("../../../assets/ui/pages/main/import-export.js"),
    include_str!("../../../assets/ui/pages/configuration/policy.js"),
    include_str!("../../../assets/ui/pages/monitoring/receipts.js"),
    include_str!("../../../assets/ui/pages/configuration/runtime-paths.js"),
    include_str!("../../../assets/ui/pages/configuration/control-settings.js"),
    include_str!("../../../assets/ui/pages/configuration/execution-layer.js"),
    include_str!("../../../assets/ui/30-shell-runtime-notify.js"),
    include_str!("../../../assets/ui/40-bootstrap.js"),
);

fn no_cache_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        content_type.parse().expect("valid content type"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        "no-store, no-cache, must-revalidate, max-age=0"
            .parse()
            .expect("valid cache control"),
    );
    headers.insert(header::PRAGMA, "no-cache".parse().expect("valid pragma"));
    headers.insert(header::EXPIRES, "0".parse().expect("valid expires"));
    headers
}

fn shell_response(page: &str) -> Response {
    let html = UI_SHELL_HTML.replace("{{ACTIVE_PAGE}}", page);
    (no_cache_headers("text/html; charset=utf-8"), html).into_response()
}

pub async fn index_overview() -> Response {
    shell_response("overview")
}

pub async fn index_approvals() -> Response {
    shell_response("approvals")
}

pub async fn index_files() -> Response {
    shell_response("files")
}

pub async fn index_policy() -> Response {
    shell_response("policy")
}

pub async fn index_receipts() -> Response {
    shell_response("receipts")
}

pub async fn index_runtime() -> Response {
    shell_response("runtime")
}

pub async fn index_settings() -> Response {
    shell_response("settings")
}

pub async fn index_execution() -> Response {
    shell_response("execution")
}

pub async fn index_approval_detail() -> Response {
    shell_response("approval-detail")
}

pub async fn index_docs() -> Redirect {
    Redirect::temporary("/help/")
}

pub async fn ui_css() -> Response {
    (no_cache_headers("text/css; charset=utf-8"), UI_CSS).into_response()
}

pub async fn design_tokens_css() -> Response {
    (
        no_cache_headers("text/css; charset=utf-8"),
        DESIGN_TOKENS_CSS,
    )
        .into_response()
}

pub async fn logo_mark_svg() -> Response {
    (no_cache_headers("image/svg+xml; charset=utf-8"), LOGO_SVG).into_response()
}

pub async fn ui_js() -> Response {
    (no_cache_headers("text/javascript; charset=utf-8"), UI_JS).into_response()
}
