//! Rejoin lines the terminal broke mid-sentence.
//!
//! A terminal hard-wraps at its column width, so one Persian sentence arrives as several
//! lines. Each fragment then resolves its own direction and alignment, and a single
//! paragraph ends up zig-zagging across the HUD. Rejoining first is what makes the rest of
//! the layout look like prose.
//!
//! Every rule here is a veto. False negatives leave a line wrapped, which is merely the
//! status quo; a false positive welds together two lines that were meant to be separate,
//! which destroys information. The rules are tuned accordingly.

use crate::bidi::RawLine;
use unicode_width::UnicodeWidthStr;

/// A wrapped line is, by construction, close to the terminal's width. Anything shorter is
/// a deliberately short line — a list of one-word items, a heading, a table cell.
const MIN_WRAP_WIDTH: usize = 40;

/// How close to the block's widest line a line must be to look like it was wrapped.
const WRAP_RATIO: f64 = 0.9;

fn ends_sentence(content: &str) -> bool {
    matches!(content.trim_end().chars().next_back(), Some('.' | '!' | '?' | ':' | '؛' | '؟' | '۔' | '。'))
}

/// Two lines can only belong to the same paragraph if they share their structure exactly.
fn same_block(a: &RawLine, b: &RawLine) -> bool {
    a.lead == b.lead
        // A table row already absorbed its own continuation lines, cell by cell. Merging
        // one into the row below would undo exactly that.
        && a.row.is_none()
        && b.row.is_none()
        && !a.boxed
        && !b.boxed
        && !a.in_code
        && !b.in_code
        && a.tail.is_empty()
        && b.tail.is_empty()
        && !a.content.trim().is_empty()
        && !b.content.trim().is_empty()
}

fn joinable(cur: &RawLine, next: &RawLine, max_width: usize) -> bool {
    let width = cur.content.width();
    width >= MIN_WRAP_WIDTH
        && (width as f64) >= WRAP_RATIO * max_width as f64
        && !ends_sentence(&cur.content)
        && cur.dir == next.dir
}

/// Merge hard-wrapped continuation lines within each paragraph.
pub fn soft_unwrap(lines: Vec<RawLine>) -> Vec<RawLine> {
    let mut out: Vec<RawLine> = Vec::with_capacity(lines.len());
    let mut i = 0usize;

    while i < lines.len() {
        let start = i;
        let mut end = i + 1;
        while end < lines.len() && same_block(&lines[start], &lines[end]) {
            end += 1;
        }

        if end - start < 2 {
            out.push(lines[start].clone());
            i = end;
            continue;
        }

        let max_width = (start..end).map(|k| lines[k].content.width()).max().unwrap_or(0);
        let mut cur = lines[start].clone();
        for line in &lines[start + 1..end] {
            if joinable(&cur, line, max_width) {
                cur.content = format!("{} {}", cur.content.trim_end(), line.content.trim_start());
            } else {
                out.push(std::mem::replace(&mut cur, line.clone()));
            }
        }
        out.push(cur);
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Dir;

    fn line(content: &str, dir: Dir) -> RawLine {
        RawLine {
            lead: String::new(),
            row: None,
            content: content.to_string(),
            tail: String::new(),
            boxed: false,
            in_code: false,
            dir,
        }
    }

    fn contents(lines: Vec<RawLine>) -> Vec<String> {
        lines.into_iter().map(|l| l.content).collect()
    }

    #[test]
    fn joins_a_hard_wrapped_persian_paragraph() {
        // Two fragments of one sentence, each near the terminal width.
        let a = "این یک متن طولانی فارسی است که توسط ترمینال در عرض هشتاد ستون شکسته";
        let b = "شده است و باید دوباره به هم بچسبد.";
        let out = contents(soft_unwrap(vec![line(a, Dir::Rtl), line(b, Dir::Rtl)]));
        assert_eq!(out, vec![format!("{a} {b}")]);
    }

    #[test]
    fn does_not_join_after_sentence_end() {
        let a = "این یک جمله کامل فارسی است که به نقطه ختم می‌شود و تمام شده است.";
        let b = "این جمله بعدی است که جدا بماند.";
        let out = contents(soft_unwrap(vec![line(a, Dir::Rtl), line(b, Dir::Rtl)]));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn does_not_join_short_list_items() {
        // The regression this guard exists for: three short lines are a list, not a paragraph.
        let out = contents(soft_unwrap(vec![
            line("سلام", Dir::Rtl),
            line("خداحافظ", Dir::Rtl),
            line("بدرود", Dir::Rtl),
        ]));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn does_not_join_across_directions() {
        let a = "این یک متن طولانی فارسی است که توسط ترمینال در عرض هشتاد ستون شکسته";
        let b = "and this english line should stay separate from it entirely";
        let out = contents(soft_unwrap(vec![line(a, Dir::Rtl), line(b, Dir::Ltr)]));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn never_joins_code_or_boxed_lines() {
        let long = "این یک متن طولانی فارسی است که توسط ترمینال در عرض هشتاد ستون شکسته";
        let mut a = line(long, Dir::Rtl);
        let mut b = line("ادامه متن", Dir::Rtl);
        a.in_code = true;
        b.in_code = true;
        assert_eq!(soft_unwrap(vec![a, b]).len(), 2);

        let mut a = line(long, Dir::Rtl);
        let mut b = line("ادامه متن", Dir::Rtl);
        a.boxed = true;
        b.boxed = true;
        assert_eq!(soft_unwrap(vec![a, b]).len(), 2);
    }

    #[test]
    fn blank_line_breaks_a_paragraph() {
        let long = "این یک متن طولانی فارسی است که توسط ترمینال در عرض هشتاد ستون شکسته";
        let out = soft_unwrap(vec![line(long, Dir::Rtl), line("", Dir::Ltr), line("ادامه", Dir::Rtl)]);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn differing_indentation_is_not_one_paragraph() {
        let long = "این یک متن طولانی فارسی است که توسط ترمینال در عرض هشتاد ستون شکسته";
        let mut a = line(long, Dir::Rtl);
        a.lead = "- ".into();
        let mut b = line("ادامه دارد", Dir::Rtl);
        b.lead = "  ".into();
        assert_eq!(soft_unwrap(vec![a, b]).len(), 2);
    }

    #[test]
    fn empty_input() {
        assert!(soft_unwrap(vec![]).is_empty());
    }
}
