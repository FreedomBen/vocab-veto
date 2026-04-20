//! M8 criterion benches. Four workloads, matching IMPLEMENTATION_PLAN §M8:
//!   1. `scan_1kib_en`           — 1 KiB English prose, strict vs substring.
//!   2. `scan_1kib_all_langs`    — 1 KiB English prose, all compiled languages.
//!   3. `scan_64kib_en`          — 64 KiB English prose, English only.
//!   4. `scan_norm_heavy`        — 1 KiB of fullwidth + NFKC-expanding input.
//!
//! Engines are built once per workload and shared across iterations so bench
//! time reflects the hot-path scan+normalize, not automaton construction.
//! Patterns come straight from the compiled `TERMS` table so the automaton
//! shape mirrors production exactly.

use std::collections::HashMap;

use banned_words_service::matcher::{compiled_langs, Engine, Lang, Mode, TERMS};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

// --- corpus builders ------------------------------------------------------

/// Repeat `seed` until the resulting string is at least `target` bytes long.
/// Does not truncate — callers that need an exact byte count must pass an
/// ASCII seed whose length divides `target` cleanly, or accept the overshoot.
fn repeat_at_least(seed: &str, target: usize) -> String {
    let mut s = String::with_capacity(target + seed.len());
    while s.len() < target {
        s.push_str(seed);
    }
    s
}

/// ~1 KiB of ASCII English prose with a handful of matchable substrings
/// seeded in so the hot path actually produces hits.
fn corpus_1kib_en() -> String {
    let seed = "The quick brown fox jumps over the lazy dog. \
                Holy shit this is a test sentence with some banned words like fuck. \
                Lorem ipsum dolor sit amet consectetur adipiscing elit. ";
    let mut s = repeat_at_least(seed, 1024);
    s.truncate(1024); // safe: seed is ASCII, every byte is a char boundary.
    s
}

/// 64 KiB of the 1 KiB English corpus, repeated.
fn corpus_64kib_en() -> String {
    corpus_1kib_en().repeat(64)
}

/// Normalization-heavy corpus: fullwidth Latin, the ﬁ ligature (U+FB01), and
/// U+FDFA (ARABIC LIGATURE SALLALLAHOU ALAYHE WASALLAM — an 18-char NFKC
/// expansion) interleaved with ASCII. Exercises the offset-map construction
/// and widen-to-source path harder than a plain ASCII run.
fn corpus_norm_heavy() -> String {
    let seed = "hello \u{FF26}\u{FF35}\u{FF23}\u{FF2B} world \u{FB01}re test \u{FDFA} end. ";
    repeat_at_least(seed, 1024)
}

// --- engine builders ------------------------------------------------------

fn engine_en() -> Engine {
    let patterns: &[&str] = TERMS
        .get("en")
        .copied()
        .expect("en is always compiled in");
    let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
    langs.insert("en".to_string(), patterns);
    Engine::new(&langs)
}

fn engine_all_langs() -> Engine {
    let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
    for (code, patterns) in TERMS.entries() {
        langs.insert((*code).to_string(), *patterns);
    }
    Engine::new(&langs)
}

// --- benches --------------------------------------------------------------

fn bench_1kib_en(c: &mut Criterion) {
    let engine = engine_en();
    let text = corpus_1kib_en();
    let scanned = vec!["en".to_string()];
    let mut g = c.benchmark_group("scan_1kib_en");
    g.throughput(Throughput::Bytes(text.len() as u64));
    g.bench_function("strict", |b| {
        b.iter(|| {
            let r = engine
                .scan(black_box(&text), black_box(&scanned), Some(Mode::Strict))
                .unwrap();
            black_box(r);
        });
    });
    g.bench_function("substring", |b| {
        b.iter(|| {
            let r = engine
                .scan(black_box(&text), black_box(&scanned), Some(Mode::Substring))
                .unwrap();
            black_box(r);
        });
    });
    g.finish();
}

fn bench_1kib_all_langs(c: &mut Criterion) {
    let engine = engine_all_langs();
    let text = corpus_1kib_en();
    let scanned: Vec<Lang> = compiled_langs().into_iter().map(String::from).collect();
    let mut g = c.benchmark_group("scan_1kib_all_langs");
    g.throughput(Throughput::Bytes(text.len() as u64));
    g.bench_function("default_mode", |b| {
        b.iter(|| {
            let r = engine
                .scan(black_box(&text), black_box(&scanned), None)
                .unwrap();
            black_box(r);
        });
    });
    g.finish();
}

fn bench_64kib_en(c: &mut Criterion) {
    let engine = engine_en();
    let text = corpus_64kib_en();
    let scanned = vec!["en".to_string()];
    let mut g = c.benchmark_group("scan_64kib_en");
    g.throughput(Throughput::Bytes(text.len() as u64));
    g.bench_function("strict", |b| {
        b.iter(|| {
            let r = engine
                .scan(black_box(&text), black_box(&scanned), Some(Mode::Strict))
                .unwrap();
            black_box(r);
        });
    });
    g.finish();
}

fn bench_norm_heavy(c: &mut Criterion) {
    let engine = engine_en();
    let text = corpus_norm_heavy();
    let scanned = vec!["en".to_string()];
    let mut g = c.benchmark_group("scan_norm_heavy");
    g.throughput(Throughput::Bytes(text.len() as u64));
    g.bench_function("substring", |b| {
        b.iter(|| {
            let r = engine
                .scan(black_box(&text), black_box(&scanned), Some(Mode::Substring))
                .unwrap();
            black_box(r);
        });
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_1kib_en,
    bench_1kib_all_langs,
    bench_64kib_en,
    bench_norm_heavy,
);
criterion_main!(benches);
