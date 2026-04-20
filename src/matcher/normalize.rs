//! NFKC + caseless normalization with a byte-level offset map back to the
//! caller's original text.
//!
//! `offset_map[i]` is the start byte of the *source codepoint* in the original
//! input that produced the normalized byte at index `i`. For a source codepoint
//! that expands to multiple normalized bytes, every normalized byte points at
//! the same source start — so the span-widening step in `scan` can map a match
//! back to a full source-codepoint range without ambiguity.
//!
//! Normalization is per-source-codepoint (NFKC + `default_case_fold` applied to
//! `once(ch)`), which is deliberately simpler than stream-NFKC: cross-codepoint
//! canonical reordering of combining marks is out of scope for v1, matching
//! DESIGN's "Known residual cases not handled in v1" carve-out.

use caseless::Caseless;
use unicode_normalization::UnicodeNormalization;

/// Post-NFKC size ceiling. DESIGN §"POST /v1/check" names this alongside the
/// 64 KiB raw-body cap; Unicode compatibility expansion can grow input up to
/// ~3×, so the post-normalization cap is larger than the raw cap.
pub const MAX_NORMALIZED_BYTES: usize = 192 * 1024;

#[derive(Debug)]
pub struct Normalized {
    pub text: String,
    pub offset_map: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizeError {
    TooLarge,
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "normalized text exceeds {} bytes", MAX_NORMALIZED_BYTES),
        }
    }
}

impl std::error::Error for NormalizeError {}

pub fn normalize(input: &str) -> Result<Normalized, NormalizeError> {
    let mut text = String::with_capacity(input.len());
    let mut offset_map: Vec<u32> = Vec::with_capacity(input.len());

    for (byte_offset, ch) in input.char_indices() {
        let start = byte_offset as u32;
        let folded = std::iter::once(ch).nfkc().default_case_fold();
        for nch in folded {
            let prev = text.len();
            text.push(nch);
            for _ in prev..text.len() {
                offset_map.push(start);
            }
            if text.len() > MAX_NORMALIZED_BYTES {
                return Err(NormalizeError::TooLarge);
            }
        }
    }

    debug_assert_eq!(text.len(), offset_map.len());
    Ok(Normalized { text, offset_map })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_identity() {
        let n = normalize("hello").unwrap();
        assert_eq!(n.text, "hello");
        assert_eq!(n.offset_map, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn ascii_case_folds() {
        let n = normalize("Hello").unwrap();
        assert_eq!(n.text, "hello");
        assert_eq!(n.offset_map, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn fullwidth_folds_to_ascii() {
        // U+FF26 FULLWIDTH LATIN CAPITAL LETTER F (3 bytes) → "f"
        let n = normalize("\u{FF26}").unwrap();
        assert_eq!(n.text, "f");
        assert_eq!(n.offset_map, vec![0]);
    }

    #[test]
    fn ligature_fi_expands_and_both_bytes_point_at_source() {
        // U+FB01 LATIN SMALL LIGATURE FI (3 bytes) → "fi" (2 bytes)
        let n = normalize("\u{FB01}").unwrap();
        assert_eq!(n.text, "fi");
        assert_eq!(n.offset_map, vec![0, 0]);
    }

    #[test]
    fn x_then_ligature_then_b_preserves_source_offsets() {
        // "x" (1) + "ﬁ" (3) + "b" (1) = 5 bytes source; normalized = "xfib"
        // offset_map: x→0, f→1 (ﬁ start), i→1 (ﬁ start), b→4
        let n = normalize("x\u{FB01}b").unwrap();
        assert_eq!(n.text, "xfib");
        assert_eq!(n.offset_map, vec![0, 1, 1, 4]);
    }

    #[test]
    fn too_large_rejected() {
        // Compose input that post-normalizes beyond 192 KiB. Each source byte
        // normalizes to itself here (ASCII), so the raw source must exceed
        // 192 KiB. We test with one byte past the cap.
        let input = "a".repeat(MAX_NORMALIZED_BYTES + 1);
        assert_eq!(normalize(&input).unwrap_err(), NormalizeError::TooLarge);
    }

    #[test]
    fn at_cap_accepted() {
        let input = "a".repeat(MAX_NORMALIZED_BYTES);
        let n = normalize(&input).unwrap();
        assert_eq!(n.text.len(), MAX_NORMALIZED_BYTES);
    }
}
