//! Criterion bench skeleton. Real workloads land in M8.

use std::collections::HashMap;

use banned_words_service::matcher::{Engine, Lang, Mode};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn scan_tiny_en(c: &mut Criterion) {
    let patterns: &[&str] = &["fuck", "shit"];
    let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
    langs.insert("en".into(), patterns);
    let engine = Engine::new(&langs);
    let text = "the quick brown fox jumps over the lazy dog".to_string();
    let scanned = vec!["en".to_string()];

    c.bench_function("scan_tiny_en_strict", |b| {
        b.iter(|| {
            let r = engine
                .scan(black_box(&text), black_box(&scanned), Some(Mode::Strict))
                .unwrap();
            black_box(r);
        });
    });
}

criterion_group!(benches, scan_tiny_en);
criterion_main!(benches);
