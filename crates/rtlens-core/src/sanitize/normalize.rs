//! Persian orthographic normalisation.
//!
//! Terminal output routinely mixes Arabic and Persian codepoints for what a reader sees as
//! the same letter. Left alone, a Persian font has no glyph for the Arabic form and the
//! renderer falls back to a different face mid-word, which looks broken even when the bidi
//! layout is correct.

use unicode_normalization::UnicodeNormalization;

/// Arabic presentation forms A and B — precomposed ligatures and positional variants that
/// should be folded back to their canonical letters.
///
/// U+FEFD/U+FEFE are unassigned and U+FEFF is the BOM, so form B stops at U+FEFC.
fn is_presentation_form(c: char) -> bool {
    matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}')
}

/// Fold an Arabic codepoint onto its Persian equivalent.
fn fold_letter(c: char) -> char {
    match c {
        '\u{064A}' => '\u{06CC}', // ARABIC YEH  -> FARSI YEH
        '\u{0649}' => '\u{06CC}', // ALEF MAKSURA -> FARSI YEH
        '\u{0643}' => '\u{06A9}', // ARABIC KAF  -> KEHEH
        other => other,
    }
}

/// Apply Persian normalisation to a whole document.
///
/// Digits are left alone on purpose: mapping ASCII digits to Persian ones would corrupt
/// version numbers, hashes and paths, which is exactly the content we work hardest to keep intact.
pub fn persian(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if is_presentation_form(ch) {
            // NFKC is applied per character, never to the whole string: a blanket NFKC pass
            // would also rewrite unrelated content such as full-width Latin in code samples.
            out.extend(ch.to_string().nfkc().map(fold_letter));
        } else {
            out.push(fold_letter(ch));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_arabic_yeh_and_kaf() {
        // "کیفیت" spelled with Arabic yeh + kaf.
        let arabic = "\u{0643}\u{064A}\u{0641}\u{064A}\u{062A}";
        let persian_form = "\u{06A9}\u{06CC}\u{0641}\u{06CC}\u{062A}";
        assert_eq!(persian(arabic), persian_form);
    }

    #[test]
    fn expands_lam_alef_ligature() {
        // U+FEFB ARABIC LIGATURE LAM WITH ALEF ISOLATED FORM -> lam + alef.
        assert_eq!(persian("\u{FEFB}"), "\u{0644}\u{0627}");
    }

    #[test]
    fn folds_positional_presentation_forms() {
        // U+FEF3 (yeh initial form) normalises to plain yeh, then folds to Farsi yeh.
        assert_eq!(persian("\u{FEF3}"), "\u{06CC}");
    }

    #[test]
    fn leaves_bom_alone() {
        // U+FEFF sits just past the presentation-form range and must not be NFKC'd here;
        // ansi::clean is what removes it.
        assert_eq!(persian("\u{FEFF}"), "\u{FEFF}");
    }

    #[test]
    fn leaves_ascii_and_digits_untouched() {
        assert_eq!(persian("v1.2.3 --flag /src/main.rs"), "v1.2.3 --flag /src/main.rs");
    }

    #[test]
    fn preserves_zwnj() {
        assert_eq!(persian("می\u{200C}رود"), "می\u{200C}رود");
    }
}
