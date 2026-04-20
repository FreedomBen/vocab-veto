//! Per-request scan: `Engine::scan` drives one `AhoCorasick` per language and
//! applies strict mode as a post-match boundary filter over the shared
//! automaton, per DESIGN §"Matching semantics" and IMPLEMENTATION_PLAN M2.

use std::collections::HashMap;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

use crate::matcher::boundary::is_word_boundary;
use crate::matcher::normalize::{normalize, NormalizeError};
use crate::matcher::{Lang, DEFAULT_MODE};

/// Maximum matches returned per request, per DESIGN §"POST /v1/check" and
/// IMPLEMENTATION_PLAN M2 item 3.
pub const MAX_MATCHES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Strict,
    Substring,
}

impl Mode {
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Substring => "substring",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub lang: Lang,
    pub term: String,
    pub matched_text: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    /// One entry per scanned language, in the caller's requested order.
    pub mode_used: Vec<(Lang, Mode)>,
    pub matches: Vec<Match>,
    pub truncated: bool,
}

pub struct Engine {
    /// Per-lang (automaton, patterns-by-pattern-id) so a `PatternID` from AC
    /// resolves back to its source term.
    automata: HashMap<Lang, (AhoCorasick, Vec<String>)>,
}

impl Engine {
    pub fn new(langs: &HashMap<Lang, &[&str]>) -> Self {
        let mut automata = HashMap::with_capacity(langs.len());
        for (lang, patterns) in langs {
            let ac = AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .build(patterns.iter().copied())
                .expect("Aho-Corasick build failed");
            let owned: Vec<String> = patterns.iter().map(|s| (*s).to_string()).collect();
            automata.insert(lang.clone(), (ac, owned));
        }
        Self { automata }
    }

    pub fn languages(&self) -> impl Iterator<Item = &Lang> {
        self.automata.keys()
    }

    pub fn has_language(&self, lang: &str) -> bool {
        self.automata.contains_key(lang)
    }

    pub fn scan(
        &self,
        text: &str,
        langs: &[Lang],
        mode: Option<Mode>,
    ) -> Result<ScanResult, NormalizeError> {
        let norm = normalize(text)?;
        let mut mode_used: Vec<(Lang, Mode)> = Vec::with_capacity(langs.len());
        let mut matches: Vec<Match> = Vec::new();
        let mut truncated = false;

        'outer: for lang in langs {
            let m = mode.unwrap_or_else(|| resolve_default_mode(lang));
            mode_used.push((lang.clone(), m));

            let Some((ac, patterns)) = self.automata.get(lang) else {
                // Unknown language — handler is expected to validate before
                // calling; defensive skip preserves mode_used order.
                continue;
            };

            for hit in ac.find_iter(&norm.text) {
                let n_start = hit.start();
                let n_end = hit.end();
                let pattern_idx = hit.pattern().as_usize();
                let term = &patterns[pattern_idx];

                let (start, end) = widen_to_source(&norm.offset_map, text, n_start, n_end);

                if m == Mode::Strict
                    && (!is_word_boundary(text, start) || !is_word_boundary(text, end))
                {
                    continue;
                }

                if matches.len() == MAX_MATCHES {
                    truncated = true;
                    break 'outer;
                }

                matches.push(Match {
                    lang: lang.clone(),
                    term: term.clone(),
                    matched_text: text[start..end].to_string(),
                    start,
                    end,
                });
            }
        }

        Ok(ScanResult {
            mode_used,
            matches,
            truncated,
        })
    }
}

/// Widen a normalized-byte match `[n_start, n_end)` into the caller's original
/// text, per DESIGN §"Mapping across NFKC expansions": each edge independently
/// snaps to the enclosing source-codepoint boundary.
fn widen_to_source(
    offset_map: &[u32],
    original: &str,
    n_start: usize,
    n_end: usize,
) -> (usize, usize) {
    debug_assert!(n_end > n_start);
    let start = offset_map[n_start] as usize;
    let last_source_cp_start = offset_map[n_end - 1] as usize;
    let cp_len = original[last_source_cp_start..]
        .chars()
        .next()
        .expect("offset_map points at a valid UTF-8 codepoint boundary")
        .len_utf8();
    (start, last_source_cp_start + cp_len)
}

/// Per-language mode lookup for the `mode = None` path. Falls back to
/// `Substring` (the most permissive default) when a language isn't keyed in
/// `DEFAULT_MODE` — this only matters during M2 when the table is a stub;
/// M3 populates every loaded language.
fn resolve_default_mode(lang: &str) -> Mode {
    DEFAULT_MODE.get(lang).copied().unwrap_or(Mode::Substring)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_en(patterns: &'static [&'static str]) -> Engine {
        let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
        langs.insert("en".into(), patterns);
        Engine::new(&langs)
    }

    fn scan_en(eng: &Engine, text: &str, mode: Option<Mode>) -> ScanResult {
        eng.scan(text, &["en".to_string()], mode).unwrap()
    }

    #[test]
    fn ascii_substring_hits_midword() {
        let eng = engine_en(&["cunt"]);
        let r = scan_en(&eng, "Scunthorpe", Some(Mode::Substring));
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].start, 1);
        assert_eq!(r.matches[0].end, 5);
        assert_eq!(r.matches[0].matched_text, "cunt");
        assert_eq!(r.mode_used, vec![("en".into(), Mode::Substring)]);
    }

    #[test]
    fn scunthorpe_strict_does_not_match() {
        let eng = engine_en(&["cunt"]);
        let r = scan_en(&eng, "Scunthorpe", Some(Mode::Strict));
        assert!(r.matches.is_empty());
        assert!(!r.truncated);
    }

    #[test]
    fn strict_matches_on_word_boundaries() {
        let eng = engine_en(&["shit"]);
        let r = scan_en(&eng, "holy shit!", Some(Mode::Strict));
        assert_eq!(r.matches.len(), 1);
        assert_eq!(&r.matches[0].matched_text, "shit");
    }

    #[test]
    fn fullwidth_evasion_folds_to_ascii() {
        let eng = engine_en(&["fuck"]);
        // FULLWIDTH LATIN CAPITAL F U C K (U+FF26, U+FF35, U+FF23, U+FF2B)
        let text = "\u{FF26}\u{FF35}\u{FF23}\u{FF2B}";
        let r = scan_en(&eng, text, Some(Mode::Substring));
        assert_eq!(r.matches.len(), 1);
        // Source span covers all four fullwidth codepoints (3 bytes each).
        assert_eq!(r.matches[0].start, 0);
        assert_eq!(r.matches[0].end, 12);
        assert_eq!(r.matches[0].matched_text, text);
    }

    #[test]
    fn ligature_fi_widens_to_source_codepoint() {
        let eng = engine_en(&["fire"]);
        // U+FB01 (ﬁ, 3 bytes) + "re" → normalized "fire" + source span covers ﬁ
        let text = "\u{FB01}re";
        let r = scan_en(&eng, text, Some(Mode::Substring));
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].start, 0);
        // ﬁ (3) + r (1) + e (1) = 5 bytes; the whole source string is one match.
        assert_eq!(r.matches[0].end, 5);
        assert_eq!(r.matches[0].matched_text, "\u{FB01}re");
    }

    #[test]
    fn asymmetric_widening_right_edge_only() {
        // "xfi" matches normalized; right edge lands on the 'i' byte of ﬁ's
        // expansion, so the end widens past the whole ﬁ source codepoint.
        // Left edge on 'x' is already on a codepoint boundary — no widening there.
        let eng = engine_en(&["xfi"]);
        let text = "x\u{FB01}b"; // "x" + "ﬁ" + "b"
        let r = scan_en(&eng, text, Some(Mode::Substring));
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].start, 0); // x at 0
        assert_eq!(r.matches[0].end, 4); // past ﬁ (1 + 3)
        assert_eq!(r.matches[0].matched_text, "x\u{FB01}");
    }

    #[test]
    fn asymmetric_widening_left_edge_only() {
        // "fib" matches normalized; left edge lands inside ﬁ's expansion, so
        // start widens back to ﬁ's start. Right edge is on 'b' — no widening.
        let eng = engine_en(&["fib"]);
        let text = "x\u{FB01}b"; // "x" + "ﬁ" + "b"
        let r = scan_en(&eng, text, Some(Mode::Substring));
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].start, 1); // ﬁ start
        assert_eq!(r.matches[0].end, 5); // past 'b' (1 + 3 + 1)
        assert_eq!(r.matches[0].matched_text, "\u{FB01}b");
    }

    #[test]
    fn cjk_substring_match() {
        let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
        langs.insert("zh".into(), &["好世"][..]);
        let eng = Engine::new(&langs);
        let r = eng
            .scan("你好世界", &["zh".to_string()], Some(Mode::Substring))
            .unwrap();
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].start, 3);
        assert_eq!(r.matches[0].end, 9);
        assert_eq!(r.matches[0].matched_text, "好世");
    }

    #[test]
    fn truncation_at_exactly_256_not_flagged() {
        // 256 matches, substring mode, pattern "a", text = 256 'a's packed.
        // LeftmostLongest gives one match per 'a' (longest is 1 byte).
        let eng = engine_en(&["a"]);
        let text = "a".repeat(256);
        let r = scan_en(&eng, &text, Some(Mode::Substring));
        assert_eq!(r.matches.len(), 256);
        assert!(!r.truncated);
    }

    #[test]
    fn truncation_at_257_flags_truncated_and_caps_at_256() {
        let eng = engine_en(&["a"]);
        let text = "a".repeat(257);
        let r = scan_en(&eng, &text, Some(Mode::Substring));
        assert_eq!(r.matches.len(), 256);
        assert!(r.truncated);
    }

    #[test]
    fn mode_used_populated_for_every_requested_lang() {
        let patterns_en: &[&str] = &["fuck"];
        let patterns_zh: &[&str] = &["好世"];
        let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
        langs.insert("en".into(), patterns_en);
        langs.insert("zh".into(), patterns_zh);
        let eng = Engine::new(&langs);
        let r = eng
            .scan(
                "abc",
                &["en".to_string(), "zh".to_string()],
                Some(Mode::Substring),
            )
            .unwrap();
        assert_eq!(
            r.mode_used,
            vec![
                ("en".into(), Mode::Substring),
                ("zh".into(), Mode::Substring),
            ]
        );
        assert!(r.matches.is_empty());
    }

    #[test]
    fn explicit_strict_on_cjk_not_clamped() {
        // Caller explicitly chooses strict on zh; mode_used echoes strict.
        let mut langs: HashMap<Lang, &[&str]> = HashMap::new();
        langs.insert("zh".into(), &["好世"][..]);
        let eng = Engine::new(&langs);
        let r = eng
            .scan("好世", &["zh".to_string()], Some(Mode::Strict))
            .unwrap();
        assert_eq!(r.mode_used, vec![("zh".into(), Mode::Strict)]);
    }

    #[test]
    fn too_large_text_bubbles_up_as_normalize_error() {
        let eng = engine_en(&["a"]);
        let text = "a".repeat(crate::matcher::MAX_NORMALIZED_BYTES + 1);
        let err = eng.scan(&text, &["en".to_string()], Some(Mode::Substring));
        assert_eq!(err.unwrap_err(), NormalizeError::TooLarge);
    }
}
