//! RTLens text engine.
//!
//! Takes raw terminal output and produces a [`Doc`]: a per-line structure with the frame
//! separated from the content, a resolved base direction, and inline runs marked for LTR
//! isolation. Deliberately free of any GUI or OS dependency so it can be tested headlessly.
//!
//! Pipeline order matters and is not arbitrary:
//!
//! 1. [`sanitize::ansi::clean`] — escape sequences and cursor games go first, because every
//!    later stage measures widths and inspects leading characters.
//! 2. [`sanitize::normalize`] — orthography, before direction is read off the letters.
//! 3. [`bidi::gutter::split`] — structure out of the way, so first-strong sees real text.
//! 4. [`bidi::direction::resolve`] — one base direction per line.
//! 5. [`table`] — recover column structure. Before unwrap, which would otherwise weld a
//!    table row to the row beneath it, and before segmentation, since each cell is
//!    segmented on its own.
//! 6. [`sanitize::unwrap`] — merge wrapped fragments; needs gutters and directions, and
//!    invalidates segmentation, so it sits between them.
//! 7. [`bidi::segment`] — inline isolation, last, on final content.

pub mod bidi;
pub mod copyback;
pub mod model;
pub mod render;
pub mod sanitize;
pub mod table;

pub use copyback::CopyMode;
pub use model::{CaptureSource, Cell, Chunk, Dir, Doc, Line, Options, Segment, Sep, TableRow};

use bidi::{direction, gutter, segment, RawLine};

/// Detect a unified diff. In diff mode every line stays LTR: the `+`/`-`/`@@` column is
/// load-bearing and a re-aligned diff is unreadable even when its prose is Persian.
fn is_diff(text: &str) -> bool {
    let mut has_hunk = false;
    let mut has_file = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            return true;
        }
        has_hunk |= line.starts_with("@@ ");
        has_file |= line.starts_with("+++ ") || line.starts_with("--- ");
    }
    has_hunk && has_file
}

fn is_fence(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Replace each detected table's lines with one line per logical row.
///
/// Rules and frame edges vanish here: they were a picture of the structure, and the
/// structure is now explicit, so the renderer draws its own. Everything outside a table is
/// passed through untouched.
fn fold_tables(raws: Vec<RawLine>, doc_dir: Dir) -> Vec<RawLine> {
    // The detector works in display columns of the original line, so hand it the line as it
    // was painted: gutter plus content. The tail is only the closing frame edge.
    let painted: Vec<String> = raws.iter().map(|r| format!("{}{}", r.lead, r.content)).collect();
    let candidates: Vec<table::Candidate> = painted
        .iter()
        .zip(&raws)
        .map(|(text, r)| table::Candidate { text, blank: text.trim().is_empty(), code: r.in_code })
        .collect();

    let tables = table::detect(&candidates);
    if tables.is_empty() {
        return raws;
    }

    let mut out: Vec<RawLine> = Vec::with_capacity(raws.len());
    let mut next = 0usize;
    for (id, found) in tables.iter().enumerate() {
        out.extend(raws[next..found.start].iter().cloned());

        // A column is aligned as a unit, and the table as a whole runs in one direction —
        // both read off the text rather than assumed, so a Latin table in a Persian
        // document still runs left to right.
        let column_dirs: Vec<Dir> = (0..found.columns)
            .map(|c| {
                let column: Vec<String> = found.rows.iter().filter_map(|r| r.cells.get(c)).cloned().collect();
                direction::document_dir(&column)
            })
            .collect();
        let every_cell: Vec<String> = found.rows.iter().flat_map(|r| r.cells.iter().cloned()).collect();
        let table_dir = direction::document_dir(&every_cell);

        for row in &found.rows {
            // The flattened text is what anything table-unaware will see, so it has to read
            // as the row did: cells in order, held apart.
            let content = row.cells.iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join("  ");
            let ctx = direction::Context { in_code: false, diff_mode: false, doc_dir };
            out.push(RawLine {
                dir: direction::resolve(&content, ctx),
                lead: String::new(),
                content,
                tail: String::new(),
                boxed: false,
                in_code: false,
                row: Some(bidi::RawRow {
                    id: id as u32,
                    columns: found.columns,
                    dir: table_dir,
                    head: row.head,
                    framed: found.framed,
                    sep: found.sep,
                    cells: row.cells.clone(),
                    align: column_dirs.clone(),
                }),
            });
        }
        next = found.end;
    }
    out.extend(raws[next..].iter().cloned());
    out
}

/// Run the full pipeline.
pub fn process(input: &str, opts: &Options, source: CaptureSource) -> Doc {
    let cleaned = sanitize::ansi::clean(input, opts.tab_width);
    let cleaned = if opts.normalize_persian { sanitize::normalize::persian(&cleaned) } else { cleaned };
    let cleaned = cleaned.trim_end_matches('\n');

    let diff_mode = is_diff(cleaned);

    // Structure first: gutters and fenced-block state, before anything reads direction.
    let mut splits: Vec<(gutter::Split, bool)> = Vec::new();
    let mut in_fence = false;
    for raw_line in cleaned.split('\n') {
        let (line_a, line_b) = match table::split_welded_rule(raw_line) {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };
        let first = line_a.as_deref().unwrap_or(raw_line);
        let split = gutter::split(first);
        let fence_delim = is_fence(&split.content);
        let in_code = in_fence || fence_delim;
        if fence_delim {
            in_fence = !in_fence;
        }
        splits.push((split, in_code));

        if let Some(second) = line_b {
            let split = gutter::split(&second);
            let fence_delim = is_fence(&split.content);
            let in_code = in_fence || fence_delim;
            if fence_delim {
                in_fence = !in_fence;
            }
            splits.push((split, in_code));
        }
    }

    // A line that is evenly balanced cannot resolve itself, so the surrounding document
    // decides — the same role the Unicode algorithm assigns to a higher-level protocol.
    let prose: Vec<String> = splits
        .iter()
        .filter(|(_, in_code)| !in_code)
        .map(|(split, _)| segment::prose_only(&split.content))
        .collect();
    let doc_dir = direction::document_dir(&prose);

    let mut raws: Vec<RawLine> = splits
        .into_iter()
        .map(|(split, in_code)| {
            let ctx = direction::Context { in_code, diff_mode, doc_dir };
            RawLine {
                dir: direction::resolve(&split.content, ctx),
                lead: split.lead,
                content: split.content,
                tail: split.tail,
                boxed: split.boxed,
                in_code,
                row: None,
            }
        })
        .collect();

    if !diff_mode {
        raws = fold_tables(raws, doc_dir);
    }

    if opts.soft_unwrap && !diff_mode {
        raws = sanitize::unwrap::soft_unwrap(raws);
    }

    let lines = raws
        .into_iter()
        .map(|r| {
            let segments = if r.content.is_empty() {
                Vec::new()
            } else if r.in_code || diff_mode {
                // Inside code, everything is code: no prose to isolate from.
                vec![Segment::Code(r.content.clone())]
            } else {
                segment::segment(&r.content, opts.strip_backticks)
            };
            let table = r.row.map(|row| model::TableRow {
                id: row.id,
                columns: row.columns,
                dir: row.dir,
                head: row.head,
                framed: row.framed,
                sep: row.sep,
                cells: row
                    .cells
                    .iter()
                    .zip(&row.align)
                    .map(|(text, align)| {
                        // A cell resolves its own direction from its own letters. Reading it
                        // off the whole row is what made a Latin cell flip to the far side of
                        // the Persian one beside it.
                        let ctx = direction::Context { in_code: false, diff_mode, doc_dir };
                        model::Cell {
                            dir: direction::resolve(text, ctx),
                            align: *align,
                            segments: segment::segment(text, opts.strip_backticks),
                        }
                    })
                    .collect(),
            });
            Line { lead: r.lead, tail: r.tail, boxed: r.boxed, code: r.in_code, dir: r.dir, segments, table }
        })
        .collect();

    Doc { lines, source, diff_mode }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(input: &str) -> Doc {
        process(input, &Options::default(), CaptureSource::Selection)
    }

    #[test]
    fn end_to_end_boxed_persian_line() {
        let d = doc("╭────────────╮\n│ فایل src/main.rs را ببین │\n╰────────────╯");
        assert_eq!(d.lines.len(), 3);

        let row = &d.lines[1];
        assert_eq!(row.dir, Dir::Rtl);
        assert!(row.boxed);
        assert_eq!(row.lead, "│ ");
        assert!(row.tail.contains('│'));
        assert!(row.segments.contains(&Segment::Code("src/main.rs".into())));

        // The frame itself is never part of the directional content.
        assert!(!row.content().contains('│'));
    }

    #[test]
    fn mixed_directions_across_lines() {
        let d = doc("سلام دنیا\nhello world\n۱۲۳");
        assert_eq!(d.lines[0].dir, Dir::Rtl);
        assert_eq!(d.lines[1].dir, Dir::Ltr);
        // Persian digits are neutral, so the line has no strong character and defaults LTR.
        assert_eq!(d.lines[2].dir, Dir::Ltr);
    }

    #[test]
    fn ansi_is_stripped_before_direction_is_read() {
        // The escape sequence must not be mistaken for leading Latin text.
        let d = doc("\x1b[31mسلام\x1b[0m");
        assert_eq!(d.lines[0].dir, Dir::Rtl);
        assert_eq!(d.lines[0].content(), "سلام");
    }

    #[test]
    fn fenced_code_block_stays_ltr() {
        let d = doc("توضیح:\n```rust\nlet x = 1; // سلام\n```\nپایان");
        assert_eq!(d.lines[0].dir, Dir::Rtl);
        assert_eq!(d.lines[2].dir, Dir::Ltr);
        assert!(d.lines[2].code);
        assert_eq!(d.lines[4].dir, Dir::Rtl);
        assert!(!d.lines[4].code);
    }

    #[test]
    fn diff_mode_pins_everything_ltr() {
        let d = doc("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-سلام\n+درود");
        assert!(d.diff_mode);
        assert!(d.lines.iter().all(|l| l.dir == Dir::Ltr));
    }

    #[test]
    fn trailing_newline_does_not_add_a_line() {
        assert_eq!(doc("one\ntwo\n").lines.len(), 2);
    }

    #[test]
    fn empty_input_yields_one_empty_line() {
        let d = doc("");
        assert_eq!(d.lines.len(), 1);
        assert!(d.lines[0].is_blank());
    }

    #[test]
    fn options_toggles_are_respected() {
        let off = Options { normalize_persian: false, soft_unwrap: false, ..Default::default() };
        let d = process("\u{0643}\u{064A}", &off, CaptureSource::Selection);
        assert_eq!(d.lines[0].content(), "\u{0643}\u{064A}");

        let on = Options::default();
        let d = process("\u{0643}\u{064A}", &on, CaptureSource::Selection);
        assert_eq!(d.lines[0].content(), "\u{06A9}\u{06CC}");
    }
}
