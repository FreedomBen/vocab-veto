//! Matcher library: compile-time term tables + runtime scan engine.
//!
//! `LIST_VERSION` and `TERMS` are pulled in from the build-script-generated
//! file in `$OUT_DIR` — see `build.rs`. The automaton map is built at startup
//! from `TERMS` (M3+); this module exposes the building blocks.

pub mod boundary;
pub mod normalize;
pub mod scan;

pub use boundary::is_word_boundary;
pub use normalize::{normalize, NormalizeError, Normalized, MAX_NORMALIZED_BYTES};
pub use scan::{Engine, Match, Mode, ScanResult, MAX_MATCHES};

/// Language code, lowercase ASCII (ISO 639-1 where available). See
/// DESIGN §"POST /v1/check" and IMPLEMENTATION_PLAN M3 item 4.
pub type Lang = String;

include!(concat!(env!("OUT_DIR"), "/generated_terms.rs"));

/// Per-language mode default. M2 ships a stub (only `en`); M3 item 4 lands
/// the full 27-entry table keyed by the LDNOOBW codes at the pinned SHA.
pub static DEFAULT_MODE: ::phf::Map<&'static str, Mode> = ::phf::phf_map! {
    "en" => Mode::Strict,
};
