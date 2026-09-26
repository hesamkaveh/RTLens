//! Recover the column structure of a table that arrived as flat lines.
//!
//! A terminal draws a table by *painting* it: every row is padded to the column widths and
//! emitted as one line. None of that structure survives a copy, so by the time RTLens sees
//! it there are only lines that happen to have spaces in the same places. Two things then
//! go wrong at once:
//!
//! 1. Each row is one bidi paragraph, so a Persian cell reverses the Latin cell beside it
//!    and the columns visibly swap places.
//! 2. A cell too long for its column was wrapped by the renderer, so one logical row is
//!    several lines and the continuation lines have no first column at all.
//!
//! Splitting the rows back into cells fixes both: every cell becomes its own paragraph
//! (problem 1), and wrapped fragments rejoin inside the cell they belong to (problem 2).
//!
//! The columns are found geometrically rather than by parsing a syntax, because there is no
//! syntax to parse — `│`-delimited output from a markdown renderer and whitespace-aligned
//! output from a report script have nothing in common except that *the same screen columns
//! stay free on every row*. That invariant is the whole detector.

use crate::model::Sep;
use unicode_width::UnicodeWidthChar;

/// Vertical rules a renderer might draw between columns.
const DELIMITERS: [char; 4] = ['│', '┃', '║', '|'];

/// The narrowest run of blank columns that counts as a deliberate column gap. A single
/// space is just a word break.
const MIN_GAP: usize = 2;

/// One logical row: continuation lines already folded into their cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<String>,
    /// True for a row above the first horizontal rule, i.e. a header.
    pub head: bool,
}

/// A detected table and the span of input lines it accounts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Row>,
    pub columns: usize,
    pub sep: Sep,
    /// True when the source drew an outer frame around the whole table.
    pub framed: bool,
    /// Indices into the slice passed to [`detect`], inclusive of `start`, exclusive of `end`.
    pub start: usize,
    pub end: usize,
}

// ------------------------------------------------------------------ column arithmetic

/// A line indexed by *display column* rather than by `char`, which is the coordinate
/// system the terminal aligned the table in. A double-width glyph occupies two columns and
/// a zero-width one (ZWNJ, a combining mark) occupies none.
struct Cols {
    /// `slots[c]` is true when column `c` is covered by a non-blank glyph.
    filled: Vec<bool>,
    /// The glyph that *starts* at each column, where one does.
    starts: Vec<Option<char>>,
    /// Every char with the column it starts at, in source order.
    chars: Vec<(usize, char)>,
}

impl Cols {
    fn new(s: &str) -> Self {
        let mut filled = Vec::new();
        let mut starts: Vec<Option<char>> = Vec::new();
        let mut chars = Vec::new();
        let mut col = 0usize;
        for c in s.chars() {
            let w = UnicodeWidthChar::width(c).unwrap_or(0);
            chars.push((col, c));
            for k in 0..w {
                filled.push(!c.is_whitespace());
                starts.push(if k == 0 { Some(c) } else { None });
            }
            col += w;
        }
        Self { filled, starts, chars }
    }

    fn width(&self) -> usize {
        self.filled.len()
    }

    /// Whether column `c` carries ink. Past the end of a line counts as blank: the row
    /// simply ended early, which is not evidence against a column boundary.
    fn filled_at(&self, c: usize) -> bool {
        self.filled.get(c).copied().unwrap_or(false)
    }

    fn char_at(&self, c: usize) -> Option<char> {
        self.starts.get(c).copied().flatten()
    }

    /// The text between two display columns, trimmed.
    fn slice(&self, range: std::ops::Range<usize>) -> String {
        self.chars
            .iter()
            .filter(|(col, _)| range.contains(col))
            .map(|(_, c)| *c)
            .collect::<String>()
            .trim()
            .to_string()
    }
}

// ------------------------------------------------------------------ classification

/// Glyphs a renderer draws rules and frames with. A line made only of these carries no
/// content, whatever else it looks like.
const RULE_CHARS: &[char] = &[
    '─', '━', '═', '╌', '┄', '┈', '╍', '-', '=', '_', '~', '┌', '┬', '┐', '├', '┼', '┤', '└', '┴', '┘', '╭',
    '╮', '╰', '╯', '╞', '╡', '╪', '╔', '╦', '╗', '╠', '╬', '╣', '╚', '╩', '╝', '│', '┃', '║', '|', '+',
];

/// The corners only an outer frame has.
const CORNERS: &[char] = &['┌', '┐', '└', '┘', '╭', '╮', '╰', '╯', '╔', '╗', '╚', '╝'];

/// A horizontal rule: the `├───┼───┤` under a header, or the `━━━━  ━━━━` a report script
/// prints. Recognised by exclusion — nothing on the line but rule and junction glyphs.
pub fn is_rule(line: &str) -> bool {
    let mut rules = 0usize;
    for c in line.chars() {
        if c.is_whitespace() {
            continue;
        }
        if !RULE_CHARS.contains(&c) {
            return false;
        }
        rules += 1;
    }
    rules >= 3
}

/// True when the line is nothing but a frame edge, i.e. a rule that also closes the box.
fn is_frame(line: &str) -> bool {
    is_rule(line) && line.chars().any(|c| CORNERS.contains(&c))
}

/// When a terminal emulator soft-wraps or screen-pads a table header line to the terminal
/// window width without emitting a newline, the following rule line can arrive welded to
/// the end of the header after a wide run of spaces.
///
/// If `line` contains non-rule text followed by a wide space gap (>= 8 spaces) and then
/// an explicit rule suffix (satisfying [`is_rule`] and containing at least 10 rule glyphs),
/// this splits them back into two lines: (text_part, rule_part).
pub fn split_welded_rule(line: &str) -> Option<(String, String)> {
    if is_rule(line) {
        return None;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            let start = i;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            let gap = i - start;
            if gap >= 8 && i < chars.len() {
                let rest: String = chars[i..].iter().collect();
                let rule_chars_count = rest.chars().filter(|c| RULE_CHARS.contains(c)).count();
                if is_rule(&rest) && rule_chars_count >= 10 {
                    let text_part: String = chars[..start].iter().collect();
                    return Some((text_part.trim_end().to_string(), rest));
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

// ------------------------------------------------------------------ boundary finding

/// Columns where a vertical rule stands on every row that reaches them.
///
/// A row that ends before the column is not evidence either way — its last cell was simply
/// empty — but a row with *text* there rules the column out immediately.
fn delimited_bounds(rows: &[&Cols], delim: char) -> Vec<usize> {
    let width = rows.iter().map(|r| r.width()).max().unwrap_or(0);
    let mut bounds = Vec::new();
    for c in 0..width {
        let mut hits = 0usize;
        let mut clean = true;
        for row in rows {
            match row.char_at(c) {
                Some(ch) if ch == delim => hits += 1,
                _ if !row.filled_at(c) => {}
                _ => {
                    clean = false;
                    break;
                }
            }
        }
        if clean && hits >= 2 {
            bounds.push(c);
        }
    }
    bounds
}

/// Internal gaps in rule lines (the whitespace between `━━━━  ━━━━` segments).
fn internal_rule_gaps(rule: &Cols) -> Vec<std::ops::Range<usize>> {
    let first_ink = rule.filled.iter().position(|&f| f).unwrap_or(0);
    let last_ink = rule.filled.iter().rposition(|&f| f).unwrap_or(0);
    let all_gaps = spaced_gaps(&[rule], &[]);
    all_gaps.into_iter().filter(|g| g.start > first_ink && g.end <= last_ink).collect()
}

/// Runs of columns left blank by every row — the gutters of a whitespace-aligned table.
///
/// When segmented rule lines (like `━━━━  ━━━━`) are present, a 1-column blank gap is
/// accepted if it coincides with a gap in the rule line, accommodating slight column
/// raggedness across wide tables.
fn spaced_gaps(rows: &[&Cols], rule_gaps: &[std::ops::Range<usize>]) -> Vec<std::ops::Range<usize>> {
    let width = rows.iter().map(|r| r.width()).max().unwrap_or(0);
    let mut gaps = Vec::new();
    let mut run: Option<usize> = None;
    for c in 0..=width {
        let blank = c < width && !rows.iter().any(|r| r.filled_at(c));
        match (blank, run) {
            (true, None) => run = Some(c),
            (false, Some(start)) => {
                let len = c - start;
                let near_rule = rule_gaps.iter().any(|rg| (start <= rg.end + 2) && (c + 2 >= rg.start));
                if len >= MIN_GAP || (len == 1 && near_rule) {
                    gaps.push(start..c);
                }
                run = None;
            }
            _ => {}
        }
    }
    gaps
}

/// Turn boundaries into the column ranges that hold the cells.
fn ranges_from_delimiters(bounds: &[usize], width: usize) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    for &b in bounds {
        ranges.push(cursor..b);
        cursor = b + 1;
    }
    ranges.push(cursor..width.max(cursor));
    ranges
}

fn ranges_from_gaps(gaps: &[std::ops::Range<usize>], width: usize) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    for gap in gaps {
        if gap.start > cursor {
            ranges.push(cursor..gap.start);
        }
        cursor = gap.end;
    }
    if cursor < width {
        ranges.push(cursor..width);
    }
    ranges
}

// ------------------------------------------------------------------ detection

/// A line as the detector sees it: the original text, minus any trailing frame.
pub struct Candidate<'a> {
    pub text: &'a str,
    pub blank: bool,
    pub code: bool,
}

/// Find every table in a run of lines.
///
/// Returns tables in input order, each carrying the line span it replaces. Lines not
/// covered by any span keep their normal handling.
pub fn detect(lines: &[Candidate<'_>]) -> Vec<Table> {
    let mut tables = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].blank || lines[i].code {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < lines.len() && !lines[end].blank && !lines[end].code {
            end += 1;
        }
        match analyse(lines, i, end) {
            Some(table) => {
                i = table.end;
                tables.push(table);
            }
            None => i = end,
        }
    }
    tables
}

/// Try to read `lines[start..end]` as a single table.
fn analyse(lines: &[Candidate<'_>], start: usize, end: usize) -> Option<Table> {
    if end - start < 2 {
        return None;
    }

    let kinds: Vec<bool> = (start..end).map(|k| is_rule(lines[k].text)).collect();
    let grids: Vec<Cols> = (start..end).map(|k| Cols::new(lines[k].text)).collect();
    // Rules are excluded from the geometry: a solid `─────` line covers every column and
    // would veto boundaries that the rows themselves agree on.
    let content: Vec<&Cols> = grids.iter().zip(&kinds).filter(|(_, rule)| !**rule).map(|(g, _)| g).collect();
    if content.len() < 2 {
        return None;
    }
    let width = content.iter().map(|g| g.width()).max().unwrap_or(0);
    let rules: Vec<&Cols> = grids.iter().zip(&kinds).filter(|(_, rule)| **rule).map(|(g, _)| g).collect();
    let rule_gaps: Vec<std::ops::Range<usize>> = rules.iter().flat_map(|r| internal_rule_gaps(r)).collect();

    // Prefer an explicit delimiter; fall back to reading the blank columns.
    let delim = DELIMITERS
        .iter()
        .copied()
        .map(|d| (d, content.iter().filter(|g| g.chars.iter().any(|(_, c)| *c == d)).count()))
        .filter(|(_, rows)| *rows >= 2)
        .max_by_key(|(_, rows)| *rows)
        .map(|(d, _)| d);

    let (sep, mut ranges) = match delim {
        Some(d) => {
            let bounds = delimited_bounds(&content, d);
            if bounds.is_empty() {
                return None;
            }
            (Sep::Delimited(d), ranges_from_delimiters(&bounds, width))
        }
        None => {
            let gaps = spaced_gaps(&content, &rule_gaps);
            if gaps.is_empty() {
                return None;
            }
            (Sep::Spaced, ranges_from_gaps(&gaps, width))
        }
    };

    // A range no row fills is the outside of the frame, not a column.
    ranges.retain(|r| content.iter().any(|g| !g.slice(r.clone()).is_empty()));
    if ranges.len() < 2 {
        return None;
    }

    let mut rows: Vec<Row> = Vec::new();
    let mut head_open = true;
    let mut saw_rule = false;
    for (offset, grid) in grids.iter().enumerate() {
        if kinds[offset] {
            // A rule closes the header and ends the current row, so the next line starts a
            // fresh one even if its first cell is empty.
            if !rows.is_empty() {
                head_open = false;
            }
            saw_rule = true;
            continue;
        }
        let cells: Vec<String> = ranges.iter().map(|r| grid.slice(r.clone())).collect();
        if cells.iter().all(String::is_empty) {
            continue;
        }
        if is_continuation(&cells, &rows, saw_rule) {
            fold_into(rows.last_mut().expect("continuation implies a previous row"), &cells);
        } else {
            rows.push(Row { cells, head: head_open && rows.is_empty() });
        }
        saw_rule = false;
    }

    // A header on its own is not a table, and neither is a shape that never fills a second
    // column on more than one row — that is prose with a wide gap in it.
    if rows.len() < 2 {
        return None;
    }
    let populated = rows.iter().filter(|r| r.cells.iter().filter(|c| !c.is_empty()).count() >= 2).count();
    if populated < 2 {
        return None;
    }

    let head = rows.first().is_some_and(|r| r.head);
    if !head {
        // Without a header rule the evidence has to come from the shape itself: every row
        // filling the same columns. One ragged row is enough to call it prose.
        let filled = |r: &Row| r.cells.iter().map(|c| !c.is_empty()).collect::<Vec<_>>();
        let first = filled(&rows[0]);
        if !rows.iter().all(|r| filled(r) == first) {
            return None;
        }
    }

    let columns = ranges.len();
    Some(Table { rows, columns, sep, framed: (start..end).any(|k| is_frame(lines[k].text)), start, end })
}

/// A row that continues the one above rather than starting its own.
///
/// The signature is a hole where the row label should be: the renderer wrapped a long cell
/// and had nothing to put in the first column. Requiring the *previous* row to have had
/// something there keeps a genuinely blank-labelled row from being swallowed.
fn is_continuation(cells: &[String], rows: &[Row], after_rule: bool) -> bool {
    if after_rule {
        return false;
    }
    let Some(prev) = rows.last() else { return false };
    if !cells[0].is_empty() || prev.cells[0].is_empty() {
        return false;
    }
    // Something has to actually continue.
    cells.iter().any(|c| !c.is_empty())
}

fn fold_into(row: &mut Row, cells: &[String]) {
    for (target, extra) in row.cells.iter_mut().zip(cells) {
        if extra.is_empty() {
            continue;
        }
        if target.is_empty() {
            target.push_str(extra);
            continue;
        }
        // A renderer breaks a long token after a hyphen or a slash, so `PBKDF2-` and
        // `HMAC-SHA256` were one word before the column was too narrow for it. Putting a
        // space back in would invent one that was never typed.
        let split_token = target.ends_with('-')
            || target.ends_with('/')
            || target.ends_with('\u{200c}')
            || extra.starts_with('/');
        if !split_token {
            target.push(' ');
        }
        target.push_str(extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pad to a display width, the way a renderer lays a cell out.
    fn pad(s: &str, width: usize) -> String {
        let w: usize = s.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum();
        format!("{s}{}", " ".repeat(width.saturating_sub(w)))
    }

    /// One painted row: cells padded to `widths`, joined the way `sep` says.
    fn row(sep: Option<char>, cells: &[&str], widths: &[usize]) -> String {
        let padded: Vec<String> = cells.iter().zip(widths).map(|(c, w)| pad(c, *w)).collect();
        match sep {
            Some(d) => format!("{d} {} {d}", padded.join(&format!(" {d} "))),
            None => format!(" {}", padded.join("  ")),
        }
    }

    fn candidates(text: &str) -> Vec<Candidate<'_>> {
        text.split('\n').map(|l| Candidate { text: l, blank: l.trim().is_empty(), code: false }).collect()
    }

    fn only(text: &str) -> Table {
        let lines = candidates(text);
        let mut found = detect(&lines);
        assert_eq!(found.len(), 1, "expected exactly one table in:\n{text}");
        found.remove(0)
    }

    fn none(text: &str) {
        let lines = candidates(text);
        assert!(detect(&lines).is_empty(), "expected no table in:\n{text}");
    }

    #[test]
    fn delimited_table_splits_into_cells() {
        let w = [9, 16];
        let text = format!(
            "{}\n{}",
            row(Some('│'), &["دکمه", "کاری که می‌کند"], &w),
            row(Some('│'), &["Plain", "متن تمیز"], &w)
        );
        let t = only(&text);
        assert_eq!(t.columns, 2);
        assert_eq!(t.sep, Sep::Delimited('│'));
        assert_eq!(t.rows[0].cells, vec!["دکمه", "کاری که می‌کند"]);
        assert_eq!(t.rows[1].cells, vec!["Plain", "متن تمیز"]);
    }

    #[test]
    fn header_rule_marks_the_header_and_disappears() {
        let t = only("│ a   │ b   │\n├─────┼─────┤\n│ یک  │ دو  │");
        assert_eq!(t.rows.len(), 2, "the rule is not a row");
        assert!(t.rows[0].head);
        assert!(!t.rows[1].head);
    }

    #[test]
    fn outer_frame_is_recognised_and_dropped() {
        let t = only("┌───────┬───────┐\n│ a     │ b     │\n│ یک    │ دو    │\n└───────┴───────┘");
        assert!(t.framed);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.columns, 2);
    }

    #[test]
    fn wrapped_cell_rejoins_inside_its_own_column() {
        let t = only(concat!(
            "│ BiDi-safe │ همان متن، به‌علاوهٔ کاراکترهای isolate یونیکد که  │\n",
            "│           │ نامرئی‌اند ولی چیدمان را حفظ می‌کنند              │\n",
            "│ MD        │ مثل Plain                                        │",
        ));
        assert_eq!(t.rows.len(), 2, "the continuation must not become its own row");
        assert_eq!(t.rows[0].cells[0], "BiDi-safe");
        assert!(t.rows[0].cells[1].contains("یونیکد که نامرئی‌اند"), "{:?}", t.rows[0].cells[1]);
        assert_eq!(t.rows[1].cells[0], "MD");
    }

    #[test]
    fn whitespace_aligned_table_is_read_from_its_blank_columns() {
        let t = only(concat!(
            " بخش      تغییر            وضعیت\n",
            "━━━━━━━  ━━━━━━━━━━━━━━━  ━━━━━━━━\n",
            " بک‌اند    تغییر نوع ستون    🔴 بله\n",
            " فرانت    حذف SVG داخلی     🟢 خیر",
        ));
        assert_eq!(t.sep, Sep::Spaced);
        assert_eq!(t.columns, 3);
        assert_eq!(t.rows.len(), 3);
        assert!(t.rows[0].head);
        assert_eq!(t.rows[2].cells[2], "🟢 خیر");
    }

    #[test]
    fn spaced_continuation_folds_into_the_cell_above() {
        let t = only(concat!(
            " بخش      تغییر                وضعیت\n",
            "━━━━━━━  ━━━━━━━━━━━━━━━━━━━  ━━━━━━━\n",
            " بک‌اند    تغییر نوع چند ستون    🔴 بله\n",
            "          شناسه از INTEGER\n",
            " فرانت    حذف SVG              🟢 خیر",
        ));
        assert_eq!(t.rows.len(), 3);
        assert!(t.rows[1].cells[1].contains("چند ستون شناسه از INTEGER"), "{:?}", t.rows[1].cells[1]);
        assert_eq!(t.rows[2].cells[0], "فرانت");
    }

    #[test]
    fn double_width_glyphs_do_not_shift_the_columns() {
        // The emoji occupies two display columns; reading by `char` would slide every
        // boundary after it one column to the left.
        let w = [10, 4];
        let text = format!("{}\n{}", row(None, &["🔴 بله", "یک"], &w), row(None, &["🟢 خیر", "دو"], &w));
        let t = only(&text);
        assert_eq!(t.columns, 2);
        assert_eq!(t.rows[0].cells, vec!["🔴 بله", "یک"]);
        assert_eq!(t.rows[1].cells, vec!["🟢 خیر", "دو"]);
    }

    #[test]
    fn zero_width_joiner_stays_with_its_word() {
        let w = [10, 4];
        let text = format!("{}\n{}", row(None, &["بک‌اند", "یک"], &w), row(None, &["فرانت‌اند", "دو"], &w));
        let t = only(&text);
        assert_eq!(t.rows[0].cells[0], "بک\u{200c}اند");
        assert_eq!(t.rows[1].cells[0], "فرانت\u{200c}اند");
    }

    #[test]
    fn a_token_broken_across_lines_rejoins_without_a_space() {
        let w = [8, 22];
        let text = format!(
            "{}\n{}\n{}",
            row(None, &["بک\u{200c}اند", "تغییر hash به PBKDF2-"], &w),
            row(None, &["", "HMAC-SHA256 با iteration"], &w),
            row(None, &["فرانت", "حذف SVG"], &w)
        );
        let t = only(&text);
        assert!(t.rows[0].cells[1].contains("PBKDF2-HMAC-SHA256"), "{:?}", t.rows[0].cells[1]);
        // A normal wrap still gets its space back.
        assert!(t.rows[0].cells[1].contains("SHA256 با"), "{:?}", t.rows[0].cells[1]);
    }

    #[test]
    fn prose_is_not_a_table() {
        none("این یک متن طولانی فارسی است که توسط ترمینال شکسته شده\nو باید دوباره به هم بچسبد.");
        none("hello world\ngoodbye world");
    }

    #[test]
    fn a_single_wide_gap_in_two_prose_lines_is_not_a_table() {
        // Ragged: the second line has nothing in the second column, so the shape is not
        // a grid and the rows must be left alone.
        none("سلام دنیا      hello\nفقط یک خط دیگر");
    }

    #[test]
    fn indented_block_is_not_a_table() {
        none("    اول\n    دوم\n    سوم");
    }

    #[test]
    fn one_row_is_not_a_table() {
        none("│ a │ b │");
    }

    #[test]
    fn blank_line_ends_a_table() {
        let text = " a    b\n یک   دو\n\n c    d\n سه   چهار";
        let lines = candidates(text);
        let found = detect(&lines);
        assert_eq!(found.len(), 2, "a blank line separates two tables");
        assert_eq!(found[0].start, 0);
        assert_eq!(found[0].end, 2);
        assert_eq!(found[1].start, 3);
    }

    #[test]
    fn rules_are_recognised_in_both_styles() {
        assert!(is_rule("├─────┼─────┤"));
        assert!(is_rule("━━━━━━  ━━━━━━"));
        assert!(is_rule("+-----+-----+"));
        assert!(is_rule("| --- | --- |"));
        assert!(!is_rule("│ a │ b │"));
        assert!(!is_rule("--"), "too short to be a rule");
    }

    #[test]
    fn split_welded_rule_splits_header_and_rule() {
        let line = "   عنوان ۱         عنوان ۲                                                               ━━━━━━━━━━━━  ━━━━━━━━━━━━";
        let split = split_welded_rule(line);
        assert!(split.is_some());
        let (head, rule) = split.unwrap();
        assert_eq!(head, "   عنوان ۱         عنوان ۲");
        assert_eq!(rule, "━━━━━━━━━━━━  ━━━━━━━━━━━━");
        assert!(is_rule(&rule));

        // Normal prose or code comments must not split
        assert!(split_welded_rule("سلام دنیا").is_none());
        assert!(split_welded_rule("let x = 1;        // ------------------").is_none());
    }

    #[test]
    fn segmented_rule_guides_ragged_columns() {
        // Here column 2 ends at col 16 and column 3 starts at col 18 in row 1, but starts
        // at col 17 in row 2. The intersection of blanks is only 1 column wide, but the
        // segmented rule line below confirms the boundary.
        let text = concat!(
            " پارامتر      شرح                                     نتیجه\n",
            "━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━\n",
            " Jc          توضیح کوتاه که به ستون بعد نزدیک شد     بله\n",
            " Jmin        توضیح دیگر                              بله\n",
        );
        let t = only(text);
        assert_eq!(t.columns, 3);
        assert_eq!(t.rows[0].cells[0], "پارامتر");
        assert_eq!(t.rows[0].cells[1], "شرح");
        assert_eq!(t.rows[0].cells[2], "نتیجه");
    }
}
