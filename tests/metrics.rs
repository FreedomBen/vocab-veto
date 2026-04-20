//! IMPLEMENTATION_PLAN M6 item 4 — scrape `/metrics` after a mixed workload
//! and assert that every DESIGN §"Metrics contract" series has the expected
//! labels and non-zero counts.
//!
//! Global state caveat: `metrics-exporter-prometheus` installs a global
//! recorder, and the `metrics!` macros resolve through it. Cargo runs each
//! file under `tests/` as its own binary, so this file owns its own recorder;
//! `tests/http.rs` does not install one (its `AppState::metrics` is `None`)
//! and therefore doesn't collide.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use tower::ServiceExt;

use banned_words_service::build_router;
use banned_words_service::matcher::{Engine, Lang, LIST_VERSION};
use banned_words_service::observability;
use banned_words_service::state::AppState;

const TEST_KEY: &str = "test-key-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn handle() -> &'static PrometheusHandle {
    static H: OnceLock<PrometheusHandle> = OnceLock::new();
    H.get_or_init(|| {
        let h = observability::install_recorder(None)
            .expect("install Prometheus recorder for metrics integration test");
        observability::record_startup(LIST_VERSION, 1, 1024);
        h
    })
}

fn state() -> Arc<AppState> {
    let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
    langs.insert("en".into(), &["fuck", "shit"][..]);
    let engine = Arc::new(Engine::new(&langs));
    Arc::new(AppState {
        engine,
        api_keys: vec![TEST_KEY.as_bytes().to_vec()],
        list_version: LIST_VERSION,
        ready: AtomicBool::new(true),
        max_inflight: 1024,
        inflight: Arc::new(AtomicUsize::new(0)),
        metrics: Some(handle().clone()),
    })
}

fn authed(method: &str, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .body(body)
        .unwrap()
}

async fn send(state: Arc<AppState>, req: Request<Body>) -> Response<Body> {
    build_router(state).oneshot(req).await.unwrap()
}

async fn text_body(resp: Response<Body>) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).expect("metrics body is UTF-8")
}

#[tokio::test]
async fn metrics_after_mixed_workload() {
    let s = state();

    // 1. Successful /v1/check — exercises matches/truncated/input-bytes,
    //    per-lang scan histogram, and RED success path.
    let ok = send(
        s.clone(),
        authed(
            "POST",
            "/v1/check",
            Body::from(r#"{"text":"holy shit!","langs":["en"],"mode":"strict"}"#),
        ),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);

    // 2. Missing-auth 401 — exercises bws_auth_failures_total{reason="missing"}
    //    and confirms RED layer sees it as 4xx despite the fast path.
    let unauth = send(
        s.clone(),
        Request::builder()
            .method("POST")
            .uri("/v1/check")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"text":"hi"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    // 3. Invalid-bearer 401 — exercises reason="invalid".
    let bad_bearer = send(
        s.clone(),
        Request::builder()
            .method("POST")
            .uri("/v1/check")
            .header("content-type", "application/json")
            .header("authorization", "Bearer not-a-real-key")
            .body(Body::from(r#"{"text":"hi"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(bad_bearer.status(), StatusCode::UNAUTHORIZED);

    // 4. /v1/languages — different endpoint label in request-duration histogram.
    let langs = send(s.clone(), authed("GET", "/v1/languages", Body::empty())).await;
    assert_eq!(langs.status(), StatusCode::OK);

    // 5. /healthz + /readyz — unauthenticated endpoints, different labels.
    let hz = send(
        s.clone(),
        Request::builder()
            .method("GET")
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(hz.status(), StatusCode::OK);

    // 6. Scrape /metrics and assert the series we care about.
    let scrape = send(
        s.clone(),
        Request::builder()
            .method("GET")
            .uri("/metrics")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(scrape.status(), StatusCode::OK);
    let body = text_body(scrape).await;

    // RED pair: request counter split by status class.
    assert!(
        body.contains(r#"bws_requests_total{status="2xx"}"#),
        "missing 2xx counter line; body:\n{body}"
    );
    assert!(
        body.contains(r#"bws_requests_total{status="4xx"}"#),
        "missing 4xx counter line; body:\n{body}"
    );

    // Request duration histogram for each endpoint hit *before* the scrape.
    // `/metrics` itself never appears — the exposition is rendered before the
    // RED layer finishes recording the current request.
    for ep in ["/v1/check", "/v1/languages", "/healthz"] {
        let needle = format!(r#"endpoint="{ep}""#);
        assert!(
            body.contains(&needle) && body.contains("bws_request_duration_seconds"),
            "missing duration histogram for endpoint {ep}; body:\n{body}"
        );
    }

    // Auth failures split by reason.
    assert!(
        body.contains(r#"bws_auth_failures_total{reason="missing"}"#),
        "missing auth-failure reason=missing"
    );
    assert!(
        body.contains(r#"bws_auth_failures_total{reason="invalid"}"#),
        "missing auth-failure reason=invalid"
    );

    // Per-language match duration for the successful strict en scan.
    assert!(
        body.contains(r#"bws_match_duration_seconds_count{lang="en",mode="strict"}"#),
        "missing per-lang match-duration histogram; body:\n{body}"
    );

    // Matches-per-request and input-bytes histograms recorded on the success.
    assert!(
        body.contains("bws_matches_per_request_count"),
        "missing matches-per-request histogram"
    );
    assert!(
        body.contains("bws_input_bytes_count"),
        "missing input-bytes histogram"
    );

    // Info + startup gauges.
    let list_version_line = format!(
        r#"bws_list_version_info{{list_version="{LIST_VERSION}"}} 1"#,
        LIST_VERSION = LIST_VERSION
    );
    assert!(
        body.contains(&list_version_line),
        "missing list-version info gauge; expected `{list_version_line}`; body:\n{body}"
    );
    assert!(
        body.contains("bws_languages_loaded 1"),
        "missing languages-loaded gauge"
    );
    // bws_inflight is snapshot on every /metrics scrape. After the workload the
    // gate guard has decremented back to 0, so the scrape reports 0 live
    // /v1/check requests.
    assert!(
        body.contains("bws_inflight 0"),
        "missing bws_inflight gauge; body:\n{body}"
    );
    assert!(
        body.contains("bws_max_inflight 1024"),
        "missing bws_max_inflight gauge"
    );
}
