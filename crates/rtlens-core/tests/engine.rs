//! Snapshot tests over realistic terminal captures.
//!
//! These assert on the rendered HTML rather than a debug dump, so a regression in what the
//! user actually sees cannot slip past a still-green internal representation.

use rtlens_core::{copyback, process, CaptureSource, CopyMode, Dir, Options};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn doc(name: &str) -> rtlens_core::Doc {
    process(&fixture(name), &Options::default(), CaptureSource::Selection)
}

macro_rules! snapshot_fixture {
    ($test:ident, $file:literal) => {
        #[test]
        fn $test() {
            insta::assert_snapshot!(rtlens_core::render::fragment(&doc($file)));
        }
    };
}

snapshot_fixture!(claude_box_persian, "claude_box_persian.txt");
snapshot_fixture!(agent_output, "agent_output.txt");
snapshot_fixture!(wrapped_prose, "wrapped_prose.txt");
snapshot_fixture!(ansi_progress, "ansi_progress.txt");
snapshot_fixture!(git_diff, "git_diff.txt");
snapshot_fixture!(code_fence, "code_fence.txt");
snapshot_fixture!(arabic_forms, "arabic_forms.txt");
snapshot_fixture!(persian_report, "persian_report.txt");
snapshot_fixture!(delimited_table, "delimited_table.txt");
snapshot_fixture!(columnar_table, "columnar_table.txt");

// Behavioural assertions that must hold regardless of how the snapshots evolve.

#[test]
fn every_box_row_keeps_its_frame_out_of_the_content() {
    let d = doc("claude_box_persian.txt");
    let rows: Vec<_> = d.lines.iter().filter(|l| l.boxed).collect();
    assert!(rows.len() >= 3, "expected the box body to be detected");
    for row in rows {
        assert!(row.lead.contains('│'), "left border belongs in the gutter");
        assert!(row.tail.contains('│'), "right border belongs in the gutter");
        assert!(!row.content().contains('│'), "frame leaked into directional content: {:?}", row.content());
    }
}

#[test]
fn persian_box_rows_resolve_rtl_and_latin_rows_resolve_ltr() {
    let d = doc("claude_box_persian.txt");
    let dirs: Vec<Dir> = d.lines.iter().filter(|l| l.boxed).map(|l| l.dir).collect();
    assert_eq!(dirs, vec![Dir::Rtl, Dir::Rtl, Dir::Ltr]);
}

#[test]
fn progress_bar_collapses_and_colour_disappears() {
    let d = doc("ansi_progress.txt");
    let text = copyback::render(&d, CopyMode::Plain);
    assert!(!text.contains('\u{1b}'), "escape sequences survived: {text:?}");
    assert!(text.contains("100% تکمیل شد"));
    assert!(!text.contains("10%"), "only the final progress frame should remain");
    assert!(text.contains("مستندات"), "OSC 8 link label must survive");
}

#[test]
fn wrapped_paragraph_is_rejoined_but_list_items_are_not() {
    let d = doc("wrapped_prose.txt");
    let text = copyback::render(&d, CopyMode::Plain);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("شکسته شده است"), "paragraph should be one line: {lines:?}");
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("- ")).count(),
        3,
        "the three list items must stay separate"
    );
}

#[test]
fn diff_stays_entirely_ltr() {
    let d = doc("git_diff.txt");
    assert!(d.diff_mode);
    assert!(d.lines.iter().all(|l| l.dir == Dir::Ltr));
}

#[test]
fn arabic_forms_are_folded_to_persian() {
    let d = doc("arabic_forms.txt");
    let text = copyback::render(&d, CopyMode::Plain);
    assert!(!text.contains('\u{064A}'), "Arabic yeh should be folded");
    assert!(!text.contains('\u{0643}'), "Arabic kaf should be folded");
    assert!(!text.contains('\u{FEFB}'), "ligature should be decomposed");
    assert!(!text.contains('\u{200F}'), "stray RLM should be gone");
}

#[test]
fn bidi_safe_copy_adds_only_isolate_controls() {
    for name in ["claude_box_persian.txt", "agent_output.txt", "wrapped_prose.txt"] {
        let d = doc(name);
        let plain = copyback::render(&d, CopyMode::Plain);
        let safe = copyback::render(&d, CopyMode::BidiSafe);
        let stripped: String =
            safe.chars().filter(|c| !matches!(*c, '\u{2066}' | '\u{2067}' | '\u{2069}')).collect();
        // Box padding is rebuilt only in plain mode, so compare on non-blank content.
        let norm = |s: &str| {
            s.lines()
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(norm(&stripped), norm(&plain), "bidi-safe changed visible text in {name}");
    }
}

#[test]
fn a_delimited_table_becomes_cells_each_with_its_own_direction() {
    // Reported from the HUD: every row was one paragraph, so the Latin label and the
    // Persian description swapped sides and the column rules landed inside the text.
    let d = doc("delimited_table.txt");
    let rows: Vec<&rtlens_core::TableRow> = d.lines.iter().filter_map(|l| l.table.as_ref()).collect();
    assert_eq!(rows.len(), 4, "header plus three rows, the frame and rules gone");
    assert!(rows[0].head);
    assert!(rows[0].framed);
    assert_eq!(rows[0].columns, 2);

    // The label column reads LTR, the description beside it RTL — the whole point.
    assert_eq!(rows[1].cells[0].text(), "Plain");
    assert_eq!(rows[1].cells[0].dir, Dir::Ltr);
    assert_eq!(rows[1].cells[1].dir, Dir::Rtl);

    for row in &rows {
        for cell in &row.cells {
            assert!(!cell.text().contains('│'), "a column rule leaked into a cell: {:?}", cell.text());
        }
    }
}

#[test]
fn a_wrapped_cell_rejoins_inside_its_own_column() {
    let d = doc("delimited_table.txt");
    let rows: Vec<&rtlens_core::TableRow> = d.lines.iter().filter_map(|l| l.table.as_ref()).collect();
    let bidi_safe = rows.iter().find(|r| r.cells[0].text() == "BiDi-safe").expect("row is present");
    let described = bidi_safe.cells[1].text();
    assert!(described.contains("یونیکد که چیدمان"), "continuation did not fold in: {described:?}");
    assert_eq!(rows.iter().filter(|r| r.cells[0].text().is_empty()).count(), 0, "no orphan rows");
}

#[test]
fn a_whitespace_aligned_table_recovers_its_four_columns() {
    let d = doc("columnar_table.txt");
    let rows: Vec<&rtlens_core::TableRow> = d.lines.iter().filter_map(|l| l.table.as_ref()).collect();
    assert_eq!(rows.len(), 3, "header plus two rows");
    assert!(rows.iter().all(|r| r.columns == 4));
    assert!(!rows[0].framed, "nothing drew a frame here");
    assert!(rows[1].cells[1].text().contains("شناسه از INTEGER به BIGINT"), "{:?}", rows[1].cells[1].text());
    assert_eq!(rows[2].cells[0].text(), "فرانت\u{200c}اند");
}

#[test]
fn a_copied_framed_table_comes_back_square() {
    // The frame is the thing that has to line up: a crooked right border is exactly the
    // failure RTLens exists to undo, so a rebuilt box must be rectangular to the column.
    let d = doc("delimited_table.txt");
    let text = copyback::render(&d, CopyMode::Plain);
    let widths: Vec<usize> = text.lines().map(unicode_width::UnicodeWidthStr::width).collect();
    assert!(widths.windows(2).all(|w| w[0] == w[1]), "frame is not square:\n{text}");
    assert!(text.starts_with('┌') && text.ends_with('┘'), "the frame was not rebuilt:\n{text}");
}

#[test]
fn a_copied_spaced_table_aligns_without_trailing_padding() {
    let d = doc("columnar_table.txt");
    let text = copyback::render(&d, CopyMode::Plain);
    assert!(text.lines().all(|l| !l.ends_with(' ')), "padding ran off the last column:\n{text}");

    // Alignment is what survives instead: every row starts its columns in the same place.
    let starts = |line: &str| {
        let mut cols = Vec::new();
        let mut at = 0usize;
        let mut blanks = 0usize;
        for c in line.chars() {
            if c == ' ' {
                blanks += 1;
            } else {
                if blanks >= 2 {
                    cols.push(at);
                }
                blanks = 0;
            }
            at += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        }
        cols
    };
    let reference = starts(text.lines().next().expect("a header"));
    for line in text.lines().skip(2) {
        assert_eq!(starts(line), reference, "column starts moved on:\n{line}\nin:\n{text}");
    }
}

#[test]
fn a_table_copies_as_markdown_pipes() {
    let d = doc("delimited_table.txt");
    let text = copyback::render(&d, CopyMode::Markdown);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("| "), "header row: {:?}", lines[0]);
    assert!(lines[1].contains("---"), "markdown needs its rule: {:?}", lines[1]);
    assert_eq!(lines.len(), 5, "four rows plus the rule:\n{text}");
}

#[test]
fn persian_lines_opening_with_a_latin_term_resolve_rtl() {
    // Reported regression: first-strong saw `merge` / `backup` and reversed whole
    // Persian sentences, throwing their bullets to the wrong side.
    let d = doc("persian_report.txt");
    let bullets: Vec<&rtlens_core::Line> =
        d.lines.iter().filter(|l| l.lead.trim_start().starts_with('-')).collect();
    assert_eq!(bullets.len(), 5, "expected five list items");
    for line in bullets {
        assert_eq!(line.dir, Dir::Rtl, "Persian bullet resolved LTR: {:?}", line.content());
    }
}
