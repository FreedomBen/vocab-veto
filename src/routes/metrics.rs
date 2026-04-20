//! GET /metrics. Unauthenticated per DESIGN §Authentication. Renders the
//! Prometheus exposition from the [`PrometheusHandle`] installed in `main`
//! via [`crate::observability::install_recorder`].

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::observability;
use crate::state::AppState;

pub async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    // Snapshot the live in-flight counter on every scrape so the gauge stays
    // current without a background ticker.
    observability::snapshot_inflight(&state.inflight);

    match state.metrics.as_ref() {
        Some(handle) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
            handle.render(),
        )
            .into_response(),
        // Unit tests may build a Router without installing a recorder.
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder not installed\n",
        )
            .into_response(),
    }
}
