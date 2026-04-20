//! Tracing + Prometheus wiring. Owns the `metrics-exporter-prometheus`
//! recorder, exposes the RED middleware layer, and owns the `/metrics` render
//! handle. Metric names and label schemes follow DESIGN §"Metrics contract".
//!
//! Startup order in `main.rs` is: `init_tracing` → `install_recorder` →
//! `record_startup` → build router with [`red_layer`] applied above auth.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use metrics::{describe_counter, describe_gauge, describe_histogram, gauge};
use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder, PrometheusHandle};

// Metric names; kept in constants so the RED middleware, the custom
// observers, and the bucket-override matchers can't drift.
pub const M_REQUESTS_TOTAL: &str = "bws_requests_total";
pub const M_AUTH_FAILURES_TOTAL: &str = "bws_auth_failures_total";
pub const M_REQUEST_DURATION: &str = "bws_request_duration_seconds";
pub const M_MATCH_DURATION: &str = "bws_match_duration_seconds";
pub const M_MATCHES_PER_REQUEST: &str = "bws_matches_per_request";
pub const M_TRUNCATED_TOTAL: &str = "bws_truncated_total";
pub const M_INPUT_BYTES: &str = "bws_input_bytes";
pub const M_LIST_VERSION_INFO: &str = "bws_list_version_info";
pub const M_LANGUAGES_LOADED: &str = "bws_languages_loaded";
pub const M_INFLIGHT: &str = "bws_inflight";
pub const M_MAX_INFLIGHT: &str = "bws_max_inflight";

/// JSON `tracing-subscriber` + env-filter. `RUST_LOG` honored; default `info`.
pub fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json())
        .init();
}

/// Install the Prometheus recorder as the global [`metrics::Recorder`], apply
/// optional bucket overrides to both latency histograms, and return the scrape
/// handle for `/metrics` to render. Callable once per process.
///
/// Bucket override applies to `bws_request_duration_seconds` **and**
/// `bws_match_duration_seconds`, per DESIGN §Metrics contract.
pub fn install_recorder(buckets: Option<&[f64]>) -> Result<PrometheusHandle, BuildError> {
    let mut builder = PrometheusBuilder::new();
    if let Some(b) = buckets {
        builder = builder
            .set_buckets_for_metric(Matcher::Full(M_REQUEST_DURATION.into()), b)?
            .set_buckets_for_metric(Matcher::Full(M_MATCH_DURATION.into()), b)?;
    }
    let handle = builder.install_recorder()?;
    describe_metrics();
    Ok(handle)
}

/// Set startup-constant metrics: the list-version info gauge, the
/// languages-loaded count, and the configured in-flight cap. Call once after
/// the engine has been built.
pub fn record_startup(list_version: &'static str, languages_loaded: usize, max_inflight: usize) {
    gauge!(M_LIST_VERSION_INFO, "list_version" => list_version).set(1.0);
    gauge!(M_LANGUAGES_LOADED).set(languages_loaded as f64);
    gauge!(M_MAX_INFLIGHT).set(max_inflight as f64);
}

/// Snapshot the live in-flight counter into the [`M_INFLIGHT`] gauge. Called
/// on every scrape via the `/metrics` handler so dashboards don't have to
/// scrape an HPA adapter-specific counter.
pub fn snapshot_inflight(inflight: &Arc<AtomicUsize>) {
    gauge!(M_INFLIGHT).set(inflight.load(Ordering::Relaxed) as f64);
}

/// RED middleware. Sits above the auth layer so fast-path 401s are recorded
/// under `status="4xx"` per DESIGN §Metrics contract. `endpoint` is pinned to
/// the known set of routes; anything else collapses to `other` to bound
/// cardinality.
pub async fn red_layer(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let endpoint = classify_endpoint(req.uri().path());
    let resp = next.run(req).await;
    let status_class = classify_status(resp.status().as_u16());
    metrics::counter!(M_REQUESTS_TOTAL, "status" => status_class).increment(1);
    metrics::histogram!(
        M_REQUEST_DURATION,
        "status" => status_class,
        "endpoint" => endpoint,
    )
    .record(start.elapsed().as_secs_f64());
    resp
}

fn classify_endpoint(path: &str) -> &'static str {
    match path {
        "/v1/check" => "/v1/check",
        "/v1/languages" => "/v1/languages",
        "/healthz" => "/healthz",
        "/readyz" => "/readyz",
        "/metrics" => "/metrics",
        _ => "other",
    }
}

fn classify_status(code: u16) -> &'static str {
    match code {
        200..=299 => "2xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        // 1xx/3xx aren't reachable from this router; bucket them under "other"
        // so we don't silently drop them if routing ever changes.
        _ => "other",
    }
}

fn describe_metrics() {
    describe_counter!(
        M_REQUESTS_TOTAL,
        "Total HTTP requests observed, bucketed by status class (2xx/4xx/5xx)."
    );
    describe_counter!(
        M_AUTH_FAILURES_TOTAL,
        "Bearer auth rejections, bucketed by reason (missing/invalid)."
    );
    describe_histogram!(
        M_REQUEST_DURATION,
        "End-to-end HTTP request duration in seconds, labelled by status class and endpoint."
    );
    describe_histogram!(
        M_MATCH_DURATION,
        "Per-language Aho-Corasick scan duration in seconds, labelled by lang and mode."
    );
    describe_histogram!(
        M_MATCHES_PER_REQUEST,
        "Match count returned per /v1/check response (pre-truncation cap: 256)."
    );
    describe_counter!(
        M_TRUNCATED_TOTAL,
        "Count of /v1/check responses where `truncated` was set to true."
    );
    describe_histogram!(
        M_INPUT_BYTES,
        "Distribution of raw `text` byte length on /v1/check requests."
    );
    describe_gauge!(
        M_LIST_VERSION_INFO,
        "Constant 1 carrying the LDNOOBW submodule SHA as its `list_version` label."
    );
    describe_gauge!(
        M_LANGUAGES_LOADED,
        "Count of languages with live Aho-Corasick automatons after startup."
    );
    describe_gauge!(
        M_INFLIGHT,
        "Current /v1/check requests in flight (counts against BWS_MAX_INFLIGHT)."
    );
    describe_gauge!(M_MAX_INFLIGHT, "Configured BWS_MAX_INFLIGHT cap.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_recorder_default_buckets_ok() {
        // Each test calls install_recorder on a fresh PrometheusBuilder that
        // doesn't touch the global recorder — we use `build_recorder` style
        // only if available. Here we just sanity-check the bucket-override
        // path compiles and accepts well-formed input.
        let buckets = [0.001_f64, 0.01, 0.1];
        let b = PrometheusBuilder::new()
            .set_buckets_for_metric(Matcher::Full(M_REQUEST_DURATION.into()), &buckets)
            .expect("request-duration buckets accepted")
            .set_buckets_for_metric(Matcher::Full(M_MATCH_DURATION.into()), &buckets)
            .expect("match-duration buckets accepted");
        // Drop the builder without installing a global recorder; we only
        // exercise the override configuration here.
        drop(b);
    }

    #[test]
    fn classify_endpoint_known_paths() {
        assert_eq!(classify_endpoint("/v1/check"), "/v1/check");
        assert_eq!(classify_endpoint("/v1/languages"), "/v1/languages");
        assert_eq!(classify_endpoint("/healthz"), "/healthz");
        assert_eq!(classify_endpoint("/readyz"), "/readyz");
        assert_eq!(classify_endpoint("/metrics"), "/metrics");
    }

    #[test]
    fn classify_endpoint_unknown_collapses() {
        assert_eq!(classify_endpoint("/"), "other");
        assert_eq!(classify_endpoint("/v1/unknown"), "other");
        assert_eq!(classify_endpoint("/v1/check/extra"), "other");
    }

    #[test]
    fn classify_status_buckets() {
        assert_eq!(classify_status(200), "2xx");
        assert_eq!(classify_status(204), "2xx");
        assert_eq!(classify_status(400), "4xx");
        assert_eq!(classify_status(401), "4xx");
        assert_eq!(classify_status(499), "4xx");
        assert_eq!(classify_status(500), "5xx");
        assert_eq!(classify_status(503), "5xx");
        assert_eq!(classify_status(301), "other");
    }
}
