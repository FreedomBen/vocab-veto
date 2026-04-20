//! Matcher library: compile-time term tables + runtime scan engine.
//!
//! `LIST_VERSION` and `TERMS` are pulled in from the build-script-generated
//! file in `$OUT_DIR` — see `build.rs`. The automaton map is built at startup
//! from `TERMS` (M3+); this module exposes the building blocks.

use std::fmt;

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

/// Sorted list of language codes compiled into the binary (the keys of
/// `TERMS`). Used as both the default load set and the error-message listing
/// when `VV_LANGS` contains an unknown code.
pub fn compiled_langs() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = TERMS.keys().copied().collect();
    v.sort_unstable();
    v
}

/// Raised at startup when `VV_LANGS` contains a code that has no compiled
/// term table. Message lists every unknown entry and every available code so
/// operators can fix the config without spelunking DESIGN.
#[derive(Debug, Clone)]
pub struct UnknownLangsError {
    pub unknown: Vec<String>,
    pub available: Vec<&'static str>,
}

impl fmt::Display for UnknownLangsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "VV_LANGS contains unknown code(s): [{}]. Compiled codes: [{}]",
            self.unknown.join(", "),
            self.available.join(", ")
        )
    }
}

impl std::error::Error for UnknownLangsError {}

/// Resolve the effective load set from `cfg.langs`. `None` returns every
/// compiled code in alphabetical order; `Some` validates each entry against
/// `TERMS` and sorts+dedups the survivors. See IMPLEMENTATION_PLAN M4.
pub fn resolve_loaded_langs(allowlist: Option<&[String]>) -> Result<Vec<Lang>, UnknownLangsError> {
    match allowlist {
        None => Ok(compiled_langs().into_iter().map(String::from).collect()),
        Some(list) => {
            let unknown: Vec<String> = list
                .iter()
                .filter(|c| !TERMS.contains_key(c.as_str()))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                return Err(UnknownLangsError {
                    unknown,
                    available: compiled_langs(),
                });
            }
            let mut out: Vec<Lang> = list.to_vec();
            out.sort();
            out.dedup();
            Ok(out)
        }
    }
}

/// Per-language mode default, keyed by the 27 LDNOOBW ISO codes at the pinned
/// SHA. `Substring` for scripts without reliable inter-word spaces (ja, ko, th,
/// zh); `Strict` for everything else. See IMPLEMENTATION_PLAN M3 item 4.
pub static DEFAULT_MODE: ::phf::Map<&'static str, Mode> = ::phf::phf_map! {
    "ar"  => Mode::Strict,
    "cs"  => Mode::Strict,
    "da"  => Mode::Strict,
    "de"  => Mode::Strict,
    "en"  => Mode::Strict,
    "eo"  => Mode::Strict,
    "es"  => Mode::Strict,
    "fa"  => Mode::Strict,
    "fi"  => Mode::Strict,
    "fil" => Mode::Strict,
    "fr"  => Mode::Strict,
    "hi"  => Mode::Strict,
    "hu"  => Mode::Strict,
    "it"  => Mode::Strict,
    "ja"  => Mode::Substring,
    "kab" => Mode::Strict,
    "ko"  => Mode::Substring,
    "nl"  => Mode::Strict,
    "no"  => Mode::Strict,
    "pl"  => Mode::Strict,
    "pt"  => Mode::Strict,
    "ru"  => Mode::Strict,
    "sv"  => Mode::Strict,
    "th"  => Mode::Substring,
    "tlh" => Mode::Strict,
    "tr"  => Mode::Strict,
    "zh"  => Mode::Substring,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_langs_sorted_and_complete() {
        let v = compiled_langs();
        assert_eq!(v.len(), TERMS.len());
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(v, sorted);
        assert!(v.contains(&"en"));
        assert!(v.contains(&"ja"));
        assert!(v.contains(&"zh"));
    }

    #[test]
    fn resolve_none_returns_every_compiled_code() {
        let out = resolve_loaded_langs(None).unwrap();
        assert_eq!(out.len(), TERMS.len());
        assert_eq!(out.first().map(String::as_str), Some("ar"));
    }

    #[test]
    fn resolve_known_subset_passes_and_sorts() {
        let allow = vec!["zh".to_string(), "en".to_string(), "ja".to_string()];
        let out = resolve_loaded_langs(Some(&allow)).unwrap();
        assert_eq!(out, vec!["en", "ja", "zh"]);
    }

    #[test]
    fn resolve_unknown_code_errors_with_listing() {
        let allow = vec!["en".to_string(), "xx".to_string(), "zz".to_string()];
        let err = resolve_loaded_langs(Some(&allow)).unwrap_err();
        assert_eq!(err.unknown, vec!["xx", "zz"]);
        let msg = err.to_string();
        assert!(msg.contains("xx"));
        assert!(msg.contains("zz"));
        assert!(msg.contains("en"));
        assert!(msg.contains("ja"));
    }

    #[test]
    fn resolve_dedups_survivors() {
        let allow = vec!["en".to_string(), "en".to_string()];
        let out = resolve_loaded_langs(Some(&allow)).unwrap();
        assert_eq!(out, vec!["en"]);
    }

    #[test]
    fn default_mode_has_entry_for_every_compiled_lang() {
        for code in compiled_langs() {
            assert!(
                DEFAULT_MODE.contains_key(code),
                "DEFAULT_MODE missing entry for compiled language `{code}`"
            );
        }
    }
}
