//! UAX #29 word-boundary check, used by strict mode as a post-match filter.

use unicode_segmentation::UnicodeSegmentation;

/// Returns true if `byte_idx` is a word boundary in `s` per UAX #29.
/// `byte_idx == 0` and `byte_idx == s.len()` are always boundaries (text edges).
pub fn is_word_boundary(s: &str, byte_idx: usize) -> bool {
    if byte_idx == 0 || byte_idx == s.len() {
        return true;
    }
    // split_word_bound_indices yields each segment's start byte; the first is
    // always 0, subsequent starts coincide with word boundaries. The trailing
    // boundary at s.len() is handled by the edge check above.
    s.split_word_bound_indices()
        .any(|(start, _)| start == byte_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_edges_are_boundaries() {
        let s = "hello";
        assert!(is_word_boundary(s, 0));
        assert!(is_word_boundary(s, s.len()));
    }

    #[test]
    fn midword_ascii_not_a_boundary() {
        let s = "scunthorpe";
        // 'c' at index 1 sits in the middle of a letter run
        assert!(!is_word_boundary(s, 1));
        // 'n'/'t' transition at index 4 still inside the word
        assert!(!is_word_boundary(s, 4));
    }

    #[test]
    fn space_separated_words_have_boundaries_around_whitespace() {
        let s = "hello world";
        // End of "hello"
        assert!(is_word_boundary(s, 5));
        // Start of "world"
        assert!(is_word_boundary(s, 6));
    }

    #[test]
    fn cjk_ideographs_are_each_their_own_word() {
        // Each CJK char is a word under UAX #29
        let s = "银行卡";
        // byte positions: 0, 3, 6, 9 (each char is 3 bytes)
        assert!(is_word_boundary(s, 0));
        assert!(is_word_boundary(s, 3));
        assert!(is_word_boundary(s, 6));
        assert!(is_word_boundary(s, 9));
    }
}
