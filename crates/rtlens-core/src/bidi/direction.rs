//! Resolve each line's base direction.
//!
//! The obvious rule — first-strong, which is what HTML's `dir="auto"` implements — is
//! wrong for the text RTLens exists to handle. A Persian sentence very often opens with an
//! English technical term:
//!
//! ```text
//! - merge قابلیت‌های پنل 6.15.3 و حل conflictها در محیط جدا: کامل
//! ```
//!
//! First-strong sees `merge`, declares the line LTR, and the whole sentence lays out
//! backwards with its bullet on the wrong side. The line is unmistakably Persian; only its
//! first word is not.
//!
//! So direction is resolved by *majority of word runs*, not by the first strong character.
//! Two details make that work where a naive ratio would not:
//!
//! * **Runs, not characters.** `فایل src/components/App.tsx را ببین` is majority-Latin by
//!   character count and would flip to LTR. By word run it is plainly Persian.
//! * **Code excluded.** A path, flag or hash is a quoted foreign object, not evidence about
//!   the language of the sentence containing it.

use crate::bidi::segment::prose_only;
use crate::model::Dir;
use unicode_bidi::{bidi_class, BidiClass};

fn strong_dir(c: char) -> Option<Dir> {
    match bidi_class(c) {
        BidiClass::L => Some(Dir::Ltr),
        BidiClass::R | BidiClass::AL => Some(Dir::Rtl),
        _ => None,
    }
}

/// First-strong lookup, kept for the places where it is still the right question.
pub fn first_strong(s: &str) -> Option<Dir> {
    s.chars().find_map(strong_dir)
}

/// Count maximal runs of same-direction strong characters: `(ltr_runs, rtl_runs)`.
///
/// Zero-width non-joiner and joiner are skipped rather than treated as separators —
/// `قابلیت‌های` is one Persian word, not two, and counting it as two would quietly bias
/// every Persian line.
pub fn count_runs(s: &str) -> (usize, usize) {
    let (mut ltr, mut rtl) = (0usize, 0usize);
    let mut current: Option<Dir> = None;
    for c in s.chars() {
        if matches!(c, '\u{200C}' | '\u{200D}') {
            continue;
        }
        match strong_dir(c) {
            Some(dir) => {
                if current != Some(dir) {
                    match dir {
                        Dir::Ltr => ltr += 1,
                        Dir::Rtl => rtl += 1,
                    }
                    current = Some(dir);
                }
            }
            None => current = None,
        }
    }
    (ltr, rtl)
}

/// Direction of a whole capture, used to settle lines that cannot decide for themselves.
///
/// This is the "higher-level protocol" the Unicode bidi algorithm defers to for paragraph
/// direction: the surrounding document knows what language it is in, and an individual
/// ambiguous line does not.
pub fn document_dir(prose_lines: &[String]) -> Dir {
    let (mut ltr, mut rtl) = (0usize, 0usize);
    for line in prose_lines {
        let (l, r) = count_runs(line);
        ltr += l;
        rtl += r;
    }
    if rtl > ltr {
        Dir::Rtl
    } else {
        Dir::Ltr
    }
}

/// Structural signals that pin a line to LTR regardless of its content.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// The line sits inside a fenced code block.
    pub in_code: bool,
    /// The whole document was detected as a unified diff.
    pub diff_mode: bool,
    /// Direction of the surrounding capture, used only to break ties.
    pub doc_dir: Dir,
}

impl Default for Context {
    fn default() -> Self {
        Self { in_code: false, diff_mode: false, doc_dir: Dir::Ltr }
    }
}

/// True for a shell prompt line, which is LTR even when the command echoes Persian.
fn looks_like_prompt(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with("$ ") || t.starts_with("PS ") || t.starts_with("➜ ") || t.starts_with("❯")
}

/// Resolve the direction of one line's content.
pub fn resolve(content: &str, ctx: Context) -> Dir {
    if ctx.diff_mode || ctx.in_code || looks_like_prompt(content) {
        return Dir::Ltr;
    }
    let (ltr, rtl) = count_runs(&prose_only(content));
    match (ltr, rtl) {
        // Nothing to go on: box borders, rules and blank lines are pure structure and stay
        // physical, so a frame never drifts to the other side of the HUD.
        (0, 0) => Dir::Ltr,
        (_, 0) => Dir::Ltr,
        (0, _) => Dir::Rtl,
        (l, r) if r > l => Dir::Rtl,
        (l, r) if l > r => Dir::Ltr,
        // A genuine tie: the document decides.
        _ => ctx.doc_dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtl_ctx() -> Context {
        Context { doc_dir: Dir::Rtl, ..Default::default() }
    }

    #[test]
    fn persian_sentence_opening_with_an_english_term_is_rtl() {
        // The reported regression: first-strong called these LTR and reversed them.
        for line in [
            "merge قابلیت‌های پنل 6.15.3 و حل conflictها در محیط جدا: کامل",
            "backup دیتابیس، ارتقای container و تست عملی بعد از migration: باقی‌مانده",
            "merge قابلیت‌ها کامل شد",
        ] {
            assert_eq!(resolve(line, rtl_ctx()), Dir::Rtl, "should be RTL: {line}");
        }
    }

    #[test]
    fn a_latin_path_does_not_drag_a_persian_line_ltr() {
        // Majority-Latin by character count, unmistakably Persian by word.
        assert_eq!(resolve("فایل src/components/App.tsx را با --verbose اجرا کن.", rtl_ctx()), Dir::Rtl);
    }

    #[test]
    fn genuinely_english_lines_stay_ltr() {
        for line in [
            "Run cargo test before you commit a1b2c3d4e5f6.",
            "added 42 packages in 3.1s",
            "Install the فایل first, then continue with the rest",
        ] {
            assert_eq!(resolve(line, rtl_ctx()), Dir::Ltr, "should be LTR: {line}");
        }
    }

    #[test]
    fn structure_only_lines_stay_physical() {
        // A box rule must not flip to the far edge just because the document is Persian.
        assert_eq!(resolve("", rtl_ctx()), Dir::Ltr);
        assert_eq!(resolve("--- 123 ---", rtl_ctx()), Dir::Ltr);
    }

    #[test]
    fn ties_defer_to_the_document() {
        let line = "merge کامل";
        assert_eq!(count_runs(&prose_only(line)), (1, 1));
        assert_eq!(resolve(line, rtl_ctx()), Dir::Rtl);
        assert_eq!(resolve(line, Context::default()), Dir::Ltr);
    }

    #[test]
    fn zwnj_does_not_split_a_persian_word() {
        assert_eq!(count_runs("قابلیت\u{200C}های"), (0, 1));
    }

    #[test]
    fn code_and_diff_context_force_ltr() {
        let line = "سلام";
        assert_eq!(resolve(line, rtl_ctx()), Dir::Rtl);
        assert_eq!(resolve(line, Context { in_code: true, ..rtl_ctx() }), Dir::Ltr);
        assert_eq!(resolve(line, Context { diff_mode: true, ..rtl_ctx() }), Dir::Ltr);
    }

    #[test]
    fn prompt_line_stays_ltr() {
        assert_eq!(resolve("$ echo سلام", rtl_ctx()), Dir::Ltr);
    }

    #[test]
    fn first_strong_still_reports_what_it_sees() {
        assert_eq!(first_strong("سلام world"), Some(Dir::Rtl));
        assert_eq!(first_strong("hello سلام"), Some(Dir::Ltr));
        assert_eq!(first_strong("123 -- ── سلام"), Some(Dir::Rtl));
        assert_eq!(first_strong(""), None);
    }

    #[test]
    fn document_dir_follows_the_majority() {
        assert_eq!(document_dir(&["سلام دنیا".into(), "خوش آمدید".into()]), Dir::Rtl);
        assert_eq!(document_dir(&["hello world".into()]), Dir::Ltr);
        assert_eq!(document_dir(&[]), Dir::Ltr);
    }
}
