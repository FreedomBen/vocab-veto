use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use banned_words_service::build_router;
use banned_words_service::config::load;
use banned_words_service::matcher::{resolve_loaded_langs, Engine, Lang, LIST_VERSION, TERMS};
use banned_words_service::observability;
use banned_words_service::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    observability::init_tracing();

    let cfg = load().inspect_err(|e| eprintln!("config error: {e}"))?;

    // Install the Prometheus recorder before touching any `metrics!` macro so
    // startup gauges land in a registered registry. The recorder is global —
    // installing once per process is correct.
    let prom_handle = observability::install_recorder(cfg.histogram_buckets.as_deref())
        .inspect_err(|e| eprintln!("metrics recorder error: {e}"))?;

    // VV_LANGS may gate loading to a subset; unset ⇒ every compiled code.
    // Unknown codes are a fatal startup error per IMPLEMENTATION_PLAN M4 item 4.
    let loaded: Vec<Lang> =
        resolve_loaded_langs(cfg.langs.as_deref()).inspect_err(|e| eprintln!("{e}"))?;

    let mut patterns: HashMap<Lang, &[&str]> = HashMap::with_capacity(loaded.len());
    for lang in &loaded {
        let terms = TERMS
            .get(lang.as_str())
            .copied()
            .expect("resolve_loaded_langs has already verified every entry is in TERMS");
        patterns.insert(lang.clone(), terms);
    }

    let engine = Arc::new(Engine::new(&patterns));
    let state = Arc::new(AppState {
        engine,
        api_keys: cfg.api_keys,
        list_version: LIST_VERSION,
        ready: AtomicBool::new(false),
        max_inflight: cfg.max_inflight,
        inflight: Arc::new(AtomicUsize::new(0)),
        metrics: Some(prom_handle),
    });
    observability::record_startup(LIST_VERSION, loaded.len(), cfg.max_inflight);
    state.ready.store(true, Ordering::Release);

    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await?;
    tracing::info!(
        target: "startup",
        addr = %cfg.listen_addr,
        list_version = LIST_VERSION,
        languages = loaded.len(),
        "Vocab Veto serving"
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
