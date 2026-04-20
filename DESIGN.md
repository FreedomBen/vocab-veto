# Banned Words Service — Design

## Goal

A high-throughput, low-latency HTTP service that answers: *"Does this string contain a banned word?"* across multiple languages. The authoritative word list is sourced from [LDNOOBW](https://github.com/LDNOOBW/List-of-Dirty-Naughty-Obscene-and-Otherwise-Bad-Words).

Target: p99 latency < 1 ms for a 1 KiB reference input on a single core; tens of thousands of RPS per instance. Larger inputs (up to the 64 KiB body limit; see API) scale linearly in the Aho-Corasick scan — the p99 target applies to the reference size, not the hard limit.

## Non-goals

- Semantic moderation, toxicity scoring, or ML-based classification.
- Admin UI for editing the list (the list is baked in at build or startup).

## Design principles

1. **Fully stateless, zero external dependencies.** No database, no Redis, no disk reads on the hot path. The entire word list is compiled into the binary via `build.rs` and loaded into Aho-Corasick automatons at startup. Pods are fungible — kill one, start another, identical state in milliseconds. Horizontal scaling is `replicas: N`.
2. **Immutable at runtime.** The word list only changes via redeploy. The image tag *is* the list version. This keeps the hot path lock-free and makes the running version trivially auditable.
3. **Hot path is allocation-free.** One `Arc<AhoCorasick>` per language, shared across all request tasks. No per-request automaton construction, no per-request heap churn beyond the match buffer.
4. **Sensible defaults, explicit overrides.** Callers can send just `{"text": "..."}` and get a correct answer. Power users can pin `langs` and `mode` per request.

## Tech stack

| Layer        | Choice                                    | Why                                                           |
| ------------ | ----------------------------------------- | ------------------------------------------------------------- |
| Language     | Rust (stable)                             | Zero-cost abstractions, predictable latency, no GC pauses.    |
| HTTP         | `axum` + `tokio`                          | Mature async stack; trivial to scale across cores.            |
| Matching     | `aho-corasick` crate                      | O(n) multi-pattern scan in a single pass over the input.      |
| Normalization| `unicode-normalization` (NFKC) + `caseless` | Handle case folding and Unicode equivalence consistently.   |
| Config       | `figment` (env + TOML)                    | 12-factor friendly; easy local overrides.                     |
| Observability| `tracing` + `tracing-subscriber`, Prometheus metrics via `axum-prometheus` | Structured logs + RED metrics out of the box. |
| Container    | Distroless static image                   | Small attack surface, fast cold start.                        |

**Alternative considered:** Go + `cloudflare/ahocorasick`. Faster to ship, meaningfully slower in published benchmarks, and GC jitter affects tail latency. Rust wins on the stated perf goal; revisit if team velocity matters more than p99.

## Data model

LDNOOBW ships one file per language (e.g. `en`, `es`, `fr`, `de`, `ja`, …). At build time we:

1. Vendor the repo as a git submodule or download pinned by commit SHA.
2. Generate a `phf`-backed map `lang -> &[&str]` via `build.rs` (compile-time, zero-alloc lookup into the term tables).
3. At startup, build one `AhoCorasick` automaton per language and store them in an `Arc<HashMap<Lang, AhoCorasick>>`. The automaton map uses `HashMap` rather than `phf` because `AhoCorasick` is not const-constructible; it is populated once at startup and read-only thereafter. Automatons are immutable and shared across all request tasks.

Memory footprint estimate: LDNOOBW is ~5k terms total across languages; the combined DFAs are well under 10 MiB.

## Matching semantics

Both **strict** (whole-word) and **substring** matching are first-class in v1. Callers pick per request via the `mode` field; when `mode` is omitted, the server applies the per-language default listed below.

- **Normalization.** Input is NFKC-normalized, then lowercased via `caseless`, in both modes. NFKC (vs NFC) is chosen deliberately — it folds compatibility forms (fullwidth, ligatures, superscripts) that are otherwise trivial evasion vectors.
- **`mode: "strict"`.** A term matches only when both edges fall on a Unicode word boundary per UAX #29. Mitigates the common **Scunthorpe problem** (banned term appearing as a substring of a safe word) for space-delimited languages. Known residual cases not handled in v1: punctuation- or hyphen-joined obfuscations (e.g., `s-h-i-t`), zero-width-joiner insertions, and leetspeak substitutions. See "Deferred to v2".
- **`mode: "substring"`.** Any Aho-Corasick hit counts. Appropriate for CJK (UAX #29 word boundaries are unreliable there) and for callers who explicitly want aggressive matching.
- **Per-language default** (applied when `mode` is omitted): `strict` for space-delimited languages (en, es, fr, de, pt, it, nl, ru, …); `substring` for CJK (ja, zh, ko). The mode chosen per language is echoed back in `mode_used` so callers can audit.
- **Span semantics.** Match `start`/`end` are byte offsets into the caller's **original** request text, suitable for direct slicing (`text[start:end]`) for redaction or highlighting. The normalizer maintains an offset map alongside the normalized buffer in a single pass, and matches are translated back before serialization. Cost: one extra `Vec<u32>` allocation per request, dwarfed by JSON parsing.
- **Mapping across NFKC expansions.** When one source codepoint expands to multiple normalized codepoints (ligatures like `ﬁ` → `fi`, compatibility forms, fullwidth letters), a match covering any portion of the expanded output maps back to the **entire source codepoint's byte range**. This is intentionally conservative: it guarantees `text[start:end]` always contains the offending content in full, at the cost of occasionally widening the reported span beyond the minimum. Spans are always on UTF-8 codepoint boundaries in the original text.

Both modes share the same automaton; the difference is a post-match boundary check, so there is no meaningful perf gap between them.

## API

### POST /v1/check

Request body (JSON, max 64 KiB — 413 `payload_too_large` above that):

```json
{
  "text": "some user input",
  "langs": ["en", "es"],
  "mode": "strict"
}
```

- `text` — string, required, non-empty.
- `langs` — array of loaded language codes, optional. Defaults to every loaded language. Codes are lowercase ASCII (ISO 639-1 where available, e.g. `en`, `ja`); inputs are case-folded before lookup so `"EN"` and `"en"` are equivalent. An empty array (`[]`) is rejected with 422 `empty_langs` — to scan every loaded language, omit the field rather than send `[]`, so the "none" vs. "all" distinction stays explicit.
- `mode` — `"strict"` | `"substring"`, optional. Omit to apply the per-language default.

**Forward compatibility.** Unknown top-level request fields are accepted and silently ignored (serde's `deny_unknown_fields` is deliberately **not** set). This lets v2 extend the schema without breaking v1 clients. The `overrides` key is reserved for future per-tenant allow/denylists; see "Deferred to v2".

Success response (200):

```json
{
  "banned": true,
  "mode_used": { "en": "strict", "ja": "substring" },
  "matches": [
    { "lang": "en", "term": "****", "matched_text": "****", "start": 12, "end": 16 }
  ],
  "truncated": false
}
```

- Each match carries both `term` — the canonical dictionary entry from LDNOOBW, stable across requests and useful for grouping, metrics, and deduplication — and `matched_text`, the exact slice of the caller's **original** input (`text[start:end]`). The two differ after NFKC folding and case folding: e.g. a term listed as `idiot` may surface with `matched_text: "Ｉｄｉｏｔ"`. `term` is a `&'static str` from the compiled table (zero cost); `matched_text` is a short heap allocation per match and is bounded by the 256-match cap.
- When `banned: false`, `matches` is `[]`. `mode_used` is always populated with an entry for every requested (or defaulted) language.
- Every `/v1/check` response — success or error — carries an `X-List-Version: <LDNOOBW commit SHA>` header. This lets each decision be audited against a specific list version without a round trip to `/readyz`, and survives proxies that rewrite bodies.
- Matches are returned in leftmost-longest, non-overlapping order, capped at the **first 256 per request in that order**. If more were found, `truncated` is `true` and the caller knows the response is a prefix of the full match list. This bounds response size on pathological input.

Error response (4xx):

```json
{ "error": "invalid_mode", "message": "mode must be 'strict' or 'substring'" }
```

| HTTP | `error`             | Condition                                         |
| ---- | ------------------- | ------------------------------------------------- |
| 400  | `bad_request`       | Malformed JSON, or missing/empty `text`.          |
| 413  | `payload_too_large` | Request body exceeds 64 KiB.                      |
| 422  | `invalid_mode`      | `mode` is present but not in the enum.            |
| 422  | `unknown_language`  | A `langs` entry is not a loaded language code.    |
| 422  | `empty_langs`       | `langs` is present but empty (`[]`).              |

### Other endpoints

- `GET /healthz` — liveness.
- `GET /readyz` — readiness (automatons loaded). Response body is `{ "ready": true, "list_version": "<LDNOOBW commit SHA>", "languages": N }` so operators and callers can audit the exact list version the running binary was built against.
- `GET /metrics` — Prometheus scrape.
- `GET /v1/languages` — list of loaded language codes and each one's default mode.

### Metrics contract

`/metrics` exposes the Prometheus series below. Label cardinality is bounded by (languages loaded × mode × status), so total series count stays in the low hundreds across a realistic deployment.

| Metric                         | Type      | Labels                   | Purpose                                                                  |
| ------------------------------ | --------- | ------------------------ | ------------------------------------------------------------------------ |
| `bws_requests_total`           | counter   | `status` (2xx / 4xx / 5xx) | Request rate and error ratio — the R and E of RED.                     |
| `bws_request_duration_seconds` | histogram | `status`                 | End-to-end latency including JSON parse and serialize.                   |
| `bws_match_duration_seconds`   | histogram | `lang`, `mode`           | Aho-Corasick scan time per (lang, mode); isolates the hot path from HTTP/JSON overhead. |
| `bws_matches_per_request`      | histogram | —                        | Distribution of match counts per request; informs tuning of the 256 cap. |
| `bws_truncated_total`          | counter   | —                        | Count of responses with `truncated: true`; expected rare, alert-worthy if not. |
| `bws_input_bytes`              | histogram | —                        | Distribution of `text` lengths in bytes; detects outlier traffic.        |
| `bws_list_version_info`        | gauge     | `sha`                    | Constant 1; lets dashboards and alerts pivot on list version across a pod fleet. |
| `bws_languages_loaded`         | gauge     | —                        | Count of automatons live after startup; sanity check for `BWS_LANGS`.    |

Histogram buckets default to the `axum-prometheus` preset tuned for sub-millisecond latency; override via env for deployments with different SLO targets.

### Why return match spans

Callers often want to redact or highlight, not just know the boolean. Returning spans costs almost nothing (Aho-Corasick produces them natively) and avoids a second round trip.

## Performance plan

- Single shared `Arc<AhoCorasick>` per language; no per-request allocation for the automaton.
- Automatons built with leftmost-longest, non-overlapping match kind — bounds match cardinality without extra filtering logic.
- Reuse a `Vec<Match>` buffer per task via a small object pool if profiling shows allocations dominate.
- Criterion benchmarks committed alongside the code; regressions fail CI.
- Load test with `oha` or `vegeta` against a representative corpus before each release.

## Deployment

- Single stateless binary, horizontally scalable.
- Container image built via `cargo chef` for fast layer caching.
- Config via env vars:
  - `BWS_LISTEN_ADDR` — HTTP listen address (e.g. `0.0.0.0:8080`).
  - `BWS_LANGS` — optional comma-separated runtime allowlist (e.g. `en,es,fr`). Defaults to every language compiled into the binary. Useful for slimming a single fat image down to a specific deployment's needs without rebuilding. A code not compiled into the binary is a **fatal startup error** (`unknown language in BWS_LANGS: xx; compiled: ...`); silent drops would mask deploy-config typos and let a pod come up serving fewer languages than the operator intended.
  - *(No `BWS_DEFAULT_MODE` — mode defaulting is per-language and defined in code, not config, so behavior is identical across deployments.)*
- **List updates ship via redeploy.** No hot reload, ever — it keeps the hot path lock-free and makes the running version trivially auditable (image tag = list version).

## Threat model and abuse posture

The service assumes it is deployed behind an authenticated API gateway or inside a trusted internal network. It does **not** perform authentication, per-caller rate limiting, request signing, or quota enforcement. The only in-process defenses against abusive traffic are:

- **64 KiB request body cap** — 413 above that.
- **256-match response cap** — `truncated: true` above that.
- **Bounded per-request work.** The Aho-Corasick scan is O(n) in input length with a constant factor independent of list size; no input can force superlinear CPU or memory.

If the service is ever exposed directly to the public internet, a gateway-level rate limit and request-size policy must be added first. Multi-tenant rate limiting is deferred to v2 and is expected to live in the gateway regardless (see below).

## Deferred to v2

- **Per-tenant allowlist / denylist overrides.** The v1 request schema will silently accept (and ignore) an `overrides` field, so adding real semantics later is non-breaking.
- **Leetspeak / homoglyph normalization.** Requires careful false-positive analysis before shipping.
- **Multi-tenant rate limiting.** Likely belongs in the gateway, not this service — revisit if that assumption breaks.

## Milestones

1. Scaffold crate, vendor LDNOOBW, build-time codegen of term tables.
2. `/v1/check` end-to-end with both `strict` and `substring` modes for `en`.
3. All LDNOOBW languages loaded; per-language mode override.
4. Metrics, health checks, container image.
5. Criterion benches + CI perf gates.
6. Load test report + v1.0 tag.
