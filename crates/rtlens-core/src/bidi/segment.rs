//! Find the runs inside a line that must be isolated LTR.
//!
//! A plain English word inside a Persian sentence already renders correctly — the bidi
//! algorithm handles strongly-typed characters well. What breaks is *neutral* characters
//! next to a directional boundary: the trailing `.` of a filename, the closing `)` of a
//! parenthetical, the `/` in a path. Those get resolved against the paragraph direction and
//! jump to the far side of the line.
//!
//! So we isolate structured tokens, not every Latin word. Over-isolating would also mean
//! rendering ordinary prose in a monospace face.

use crate::model::Segment;
use once_cell::sync::Lazy;
use regex::Regex;

static CODE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"`[^`\n]+`",
        r"|(?:https?|ftp|file)://[^\s]+",
        r"|[A-Za-z]:\\[^\s]+",
        r"|(?:~|\.{1,2})?/[A-Za-z0-9._@+-]+(?:/[A-Za-z0-9._@+-]*)*",
        r"|[A-Za-z0-9._@+-]+(?:/[A-Za-z0-9._@+-]+)+",
        r"|--?[A-Za-z][A-Za-z0-9-]*(?:=[^\s]+)?",
        // Environment variables and key-value assignments (e.g. `PLATFORM_ENV=stg`, `report.maxPositions=50000`).
        r"|[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.|->)[A-Za-z_][A-Za-z0-9_]*)*(?:\(\))?=[A-Za-z0-9._/+-]+",
        // Scoped identifiers and method calls (e.g. `foo::bar`, `std::collections::HashMap`, `obj->method()`).
        r"|[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.|->)[A-Za-z_][A-Za-z0-9_]*)+(?:\(\))?",
        // Filenames with extensions, including hyphens (e.g. `docker-compose.yml`, `package.json`).
        r"|[A-Za-z_][A-Za-z0-9_-]*(?:\.[A-Za-z0-9_-]+)+",
        // Snake_case and uppercase identifiers (e.g. `dockhand_stg_webhook_url`, `MAX_RETRIES`).
        r"|_*[A-Za-z0-9]+(?:_+[A-Za-z0-9]+)+_*|__[A-Za-z0-9]+__",
        // Email addresses (e.g. `admin@local.c`, `user@example.com`).
        r"|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z0-9]+",
        r"|v?[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:[-+][A-Za-z0-9.]+)?",
        r"|[0-9a-f]{7,40}",
        r"|\[[\x21-\x7E][\x20-\x7E]*\]",
    ))
    .unwrap()
});

/// A token may only start at a word boundary. The `regex` crate has no lookbehind, so this
/// is enforced after the fact — it is what stops `e-mail` from yielding a `-mail` "flag".
fn at_token_boundary(hay: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    match hay[..start].chars().next_back() {
        None => true,
        Some(prev) => {
            prev.is_whitespace()
                || matches!(prev, '(' | '[' | '{' | '"' | '\'' | '«' | '،' | ',' | ':' | '=' | '|')
        }
    }
}

/// True when `closer` at the end of `s` has a matching opener inside `s`.
fn balanced(s: &str, closer: char) -> bool {
    let opener = match closer {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => return true,
    };
    let opens = s.chars().filter(|&c| c == opener).count();
    let closes = s.chars().filter(|&c| c == closer).count();
    opens >= closes
}

/// Push sentence punctuation back out of a token.
///
/// `فایل main.rs.` must isolate `main.rs` and leave the final period in the RTL run,
/// otherwise the period renders at the wrong end of the sentence. A closing bracket is
/// only pushed out when it has no matching opener inside the token.
fn trim_trailing_punct(s: &str) -> &str {
    let mut end = s.len();
    while end > 0 {
        let sub = &s[..end];
        let Some(c) = sub.chars().next_back() else { break };
        let strip = match c {
            '.' | ',' | ':' | ';' | '!' | '?' | '،' | '؛' | '؟' | '»' => true,
            ')' | ']' | '}' => !balanced(sub, c),
            _ => false,
        };
        if !strip {
            break;
        }
        end -= c.len_utf8();
    }
    &s[..end]
}

/// Reject matches that are almost certainly ordinary words.
fn acceptable(token: &str) -> bool {
    if token.chars().count() < 2 {
        return false;
    }
    // A run of 7-40 hex letters with no digit is a word like "defaced", not a commit hash.
    let all_hex = token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if all_hex && !token.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

fn push_text(segs: &mut Vec<Segment>, text: &str) {
    if text.is_empty() {
        return;
    }
    // Merge with a preceding text run so rejected matches rejoin their neighbours.
    if let Some(Segment::Text(prev)) = segs.last_mut() {
        prev.push_str(text);
    } else {
        segs.push(Segment::Text(text.to_string()));
    }
}

/// Byte ranges of `content` that are structured code tokens.
///
/// Shared by [`segment`] and [`prose_only`] so that what gets isolated for rendering and
/// what gets discounted when resolving direction can never disagree.
fn code_ranges(content: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut last = 0usize;
    for m in CODE_RE.find_iter(content) {
        if m.start() < last || !at_token_boundary(content, m.start()) {
            continue;
        }
        let kept = trim_trailing_punct(m.as_str());
        if kept.is_empty() || !acceptable(kept) {
            continue;
        }
        let end = m.start() + kept.len();
        ranges.push(m.start()..end);
        last = end;
    }
    ranges
}

/// The line with code tokens blanked out.
///
/// Direction is a statement about what language a line is *written in*, and a path, flag
/// or hash says nothing about that — it is a quoted foreign object that happens to be
/// spelled in Latin script. Counting it would drag Persian sentences to LTR purely for
/// mentioning a filename.
pub fn prose_only(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut last = 0usize;
    for range in code_ranges(content) {
        out.push_str(&content[last..range.start]);
        out.push(' ');
        last = range.end;
    }
    out.push_str(&content[last..]);
    out
}

/// Split line content into text and isolated-code runs.
pub fn segment(content: &str, strip_backticks: bool) -> Vec<Segment> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut segs: Vec<Segment> = Vec::new();
    let mut last = 0usize;

    for range in code_ranges(content) {
        if range.start > last {
            push_text(&mut segs, &content[last..range.start]);
        }
        let kept = &content[range.clone()];
        let code = if strip_backticks && kept.len() >= 2 && kept.starts_with('`') && kept.ends_with('`') {
            &kept[1..kept.len() - 1]
        } else {
            kept
        };
        segs.push(Segment::Code(code.to_string()));
        last = range.end;
    }
    if last < content.len() {
        push_text(&mut segs, &content[last..]);
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(content: &str) -> Vec<(&'static str, String)> {
        segment(content, true)
            .into_iter()
            .map(|s| match s {
                Segment::Text(t) => ("text", t),
                Segment::Code(t) => ("code", t),
            })
            .collect()
    }

    fn codes(content: &str) -> Vec<String> {
        kinds(content).into_iter().filter(|(k, _)| *k == "code").map(|(_, t)| t).collect()
    }

    #[test]
    fn a_settings_key_keeps_its_value() {
        // Reported from a Codex report table: the value broke away from its key and its
        // digits reordered against the Persian around them.
        let segs = segment("محدودیت پیش‌فرض report.maxPositions=50000 است", true);
        assert!(
            segs.contains(&Segment::Code("report.maxPositions=50000".into())),
            "key and value must stay one token: {segs:?}"
        );
    }

    #[test]
    fn isolates_paths_flags_and_urls() {
        assert_eq!(codes("فایل /src/main.rs را ببین"), vec!["/src/main.rs"]);
        assert_eq!(codes("از --help استفاده کن"), vec!["--help"]);
        assert_eq!(codes("برو به https://example.com/docs"), vec!["https://example.com/docs"]);
        assert_eq!(codes("مسیر src/components/App.tsx"), vec!["src/components/App.tsx"]);
    }

    #[test]
    fn trailing_period_stays_with_the_sentence() {
        // The whole point: the period must not be swallowed into the LTR run.
        let out = kinds("فایل main.rs.");
        assert_eq!(
            out,
            vec![("text", "فایل ".to_string()), ("code", "main.rs".to_string()), ("text", ".".to_string()),]
        );
    }

    #[test]
    fn unmatched_closing_bracket_is_pushed_out() {
        let out = kinds("(فایل main.rs)");
        assert_eq!(out.last().unwrap(), &("text", ")".to_string()));
    }

    #[test]
    fn balanced_brackets_stay_inside() {
        assert_eq!(codes("مقدار [INFO] است"), vec!["[INFO]"]);
    }

    #[test]
    fn hyphenated_word_is_not_a_flag() {
        assert!(codes("send an e-mail today").is_empty());
    }

    #[test]
    fn hex_word_without_digits_is_not_a_hash() {
        assert!(codes("the defaced page").is_empty());
        assert_eq!(codes("commit a1b2c3d4e5"), vec!["a1b2c3d4e5"]);
    }

    #[test]
    fn versions_and_identifiers() {
        assert_eq!(codes("نسخه v1.2.3 منتشر شد"), vec!["v1.2.3"]);
        assert_eq!(codes("فایل package.json را باز کن"), vec!["package.json"]);
        assert_eq!(codes("فایل docker-compose.yml را باز کن"), vec!["docker-compose.yml"]);
        assert_eq!(codes("تابع foo::bar را صدا بزن"), vec!["foo::bar"]);
        assert_eq!(codes("متغیر dockhand_stg_webhook_url را تنظیم کن"), vec!["dockhand_stg_webhook_url"]);
        assert_eq!(codes("مقدار PLATFORM_ENV=production است"), vec!["PLATFORM_ENV=production"]);
        assert_eq!(codes("کاربر admin@local.c را بساز"), vec!["admin@local.c"]);
    }

    #[test]
    fn backticks_are_stripped_when_asked() {
        assert_eq!(segment("مقدار `cargo test` است", true)[1], Segment::Code("cargo test".into()));
        assert_eq!(segment("مقدار `cargo test` است", false)[1], Segment::Code("`cargo test`".into()));
    }

    #[test]
    fn plain_prose_is_left_alone() {
        assert_eq!(kinds("این یک جمله فارسی است"), vec![("text", "این یک جمله فارسی است".to_string())]);
        assert_eq!(
            kinds("just a plain english line"),
            vec![("text", "just a plain english line".to_string())]
        );
    }

    #[test]
    fn empty_content() {
        assert!(segment("", true).is_empty());
    }

    #[test]
    fn prose_only_drops_code_tokens() {
        assert_eq!(
            prose_only("فایل src/components/App.tsx را با --verbose اجرا کن.").trim(),
            "فایل   را با   اجرا کن."
        );
        assert_eq!(prose_only("این یک جمله فارسی است"), "این یک جمله فارسی است");
    }

    #[test]
    fn segments_reassemble_to_the_original() {
        for line in [
            "فایل /src/main.rs را با --verbose اجرا کن.",
            "run cargo build --release then check target/debug",
            "نسخه v1.2.3 در package.json ثبت شد (مهم)",
        ] {
            let joined: String = segment(line, false).iter().map(|s| s.text()).collect();
            assert_eq!(joined, line, "lossless reassembly failed for: {line}");
        }
    }
}
