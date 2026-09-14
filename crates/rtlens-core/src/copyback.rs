//! Turn an analysed [`Doc`] back into text worth pasting somewhere else.

use crate::model::{Cell, Chunk, Dir, Doc, Line, Segment, Sep, TableRow};
use unicode_width::UnicodeWidthStr;

/// Unicode bidi isolate controls. Unlike the deprecated embedding controls (LRE/RLE/PDF),
/// isolates do not leak their direction into surrounding text, which is exactly the
/// property needed for a fragment that will be pasted into unknown context.
const LRI: char = '\u{2066}';
const RLI: char = '\u{2067}';
const PDI: char = '\u{2069}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// Clean text, nothing added. Best for pasting back into a terminal or an editor.
    Plain,
    /// Text carrying explicit isolate controls, so the layout survives being pasted into
    /// Slack, Telegram, Notion or a GitHub comment — apps that run their own bidi pass.
    BidiSafe,
    /// Plain, with inline code runs re-fenced in backticks.
    Markdown,
}

fn body(line: &Line, backtick_code: bool) -> String {
    let mut s = String::new();
    for seg in &line.segments {
        match seg {
            Segment::Text(t) => s.push_str(t),
            Segment::Code(t) if backtick_code => {
                s.push('`');
                s.push_str(t);
                s.push('`');
            }
            Segment::Code(t) => s.push_str(t),
        }
    }
    s
}

// ------------------------------------------------------------------------------ tables

/// The glyphs a rebuilt frame is drawn with, matched to the ones the capture used so a
/// light table does not come back heavy.
struct Frame {
    v: char,
    h: char,
    top: [char; 3],
    mid: [char; 3],
    bot: [char; 3],
}

fn frame_for(delim: char) -> Frame {
    match delim {
        '┃' => Frame {
            v: '┃', h: '━', top: ['┏', '┳', '┓'], mid: ['┣', '╋', '┫'], bot: ['┗', '┻', '┛']
        },
        '║' => Frame {
            v: '║', h: '═', top: ['╔', '╦', '╗'], mid: ['╠', '╬', '╣'], bot: ['╚', '╩', '╝']
        },
        '|' => Frame { v: '|', h: '-', top: ['+', '+', '+'], mid: ['+', '+', '+'], bot: ['+', '+', '+'] },
        _ => Frame {
            v: '│', h: '─', top: ['┌', '┬', '┐'], mid: ['├', '┼', '┤'], bot: ['└', '┴', '┘']
        },
    }
}

fn rule_line(widths: &[usize], h: char, ends: [char; 3]) -> String {
    let mut out = String::new();
    out.push(ends[0]);
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            out.push(ends[1]);
        }
        out.extend(std::iter::repeat_n(h, w + 2));
    }
    out.push(ends[2]);
    out
}

/// One cell as text, plus the width it will actually occupy — isolate controls are
/// invisible, so padding has to be measured on the text without them.
fn cell_text(cell: &Cell, backtick_code: bool, isolate: bool) -> (String, usize) {
    let mut visible = String::new();
    let mut out = String::new();
    if isolate {
        out.push(if cell.dir == Dir::Rtl { RLI } else { LRI });
    }
    for seg in &cell.segments {
        match seg {
            Segment::Text(t) => {
                out.push_str(t);
                visible.push_str(t);
            }
            Segment::Code(t) => {
                if backtick_code {
                    out.push('`');
                    visible.push('`');
                }
                if isolate {
                    out.push(LRI);
                }
                out.push_str(t);
                visible.push_str(t);
                if isolate {
                    out.push(PDI);
                }
                if backtick_code {
                    out.push('`');
                    visible.push('`');
                }
            }
        }
    }
    if isolate {
        out.push(PDI);
    }
    (out, visible.width())
}

fn column_widths(rows: &[&TableRow], backtick_code: bool) -> Vec<usize> {
    let columns = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    (0..columns)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.cells.get(c))
                .map(|cell| cell_text(cell, backtick_code, false).1)
                .max()
                .unwrap_or(0)
        })
        .collect()
}

/// Rebuild a table as text.
///
/// Padded to square, because the recovered cells are what the reader just looked at — a
/// table that pastes back ragged would make the fix feel undone. Markdown gets a pipe
/// table, which is the one form every renderer downstream understands.
fn table_text(rows: &[&TableRow], mode: CopyMode) -> String {
    let backtick = matches!(mode, CopyMode::Markdown);
    let isolate = matches!(mode, CopyMode::BidiSafe);
    let widths = column_widths(rows, backtick);
    if widths.is_empty() {
        return String::new();
    }

    let cells_of = |row: &TableRow| -> Vec<String> {
        (0..widths.len())
            .map(|c| {
                let (text, w) = match row.cells.get(c) {
                    Some(cell) => cell_text(cell, backtick, isolate),
                    None => (String::new(), 0),
                };
                format!("{text}{}", " ".repeat(widths[c].saturating_sub(w)))
            })
            .collect()
    };

    let mut out: Vec<String> = Vec::new();
    let markdown_rule = |sep: &str| {
        let dashes: Vec<String> = widths.iter().map(|w| "-".repeat((*w).max(3))).collect();
        format!("{sep} {} {sep}", dashes.join(&format!(" {sep} ")))
    };

    match (mode, rows[0].sep) {
        // A pipe table is the portable form, so markdown always emits one whatever the
        // capture looked like. Markdown requires the rule, header row or not.
        (CopyMode::Markdown, _) => {
            for (i, row) in rows.iter().enumerate() {
                out.push(format!("| {} |", cells_of(row).join(" | ")));
                if i == 0 {
                    out.push(markdown_rule("|"));
                }
            }
        }
        (_, Sep::Delimited(delim)) => {
            let f = frame_for(delim);
            if rows[0].framed {
                out.push(rule_line(&widths, f.h, f.top));
            }
            for (i, row) in rows.iter().enumerate() {
                let joined = cells_of(row).join(&format!(" {} ", f.v));
                out.push(format!("{} {joined} {}", f.v, f.v));
                if i == 0 && row.head {
                    out.push(rule_line(&widths, f.h, f.mid));
                }
            }
            if rows[0].framed {
                out.push(rule_line(&widths, f.h, f.bot));
            }
        }
        (_, Sep::Spaced) => {
            for (i, row) in rows.iter().enumerate() {
                out.push(cells_of(row).join("  ").trim_end().to_string());
                if i == 0 && row.head {
                    let bars: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                    out.push(bars.join("  "));
                }
            }
        }
    }
    out.join("\n")
}

/// Width of the widest boxed row, used to rebuild a square frame.
fn box_body_width(doc: &Doc, backtick_code: bool) -> usize {
    doc.lines
        .iter()
        .filter(|l| l.boxed)
        .map(|l| format!("{}{}", l.lead, body(l, backtick_code)).width())
        .max()
        .unwrap_or(0)
}

fn plainish(doc: &Doc, mode: CopyMode) -> String {
    let backtick_code = matches!(mode, CopyMode::Markdown);
    let target = box_body_width(doc, backtick_code);
    let mut out: Vec<String> = Vec::new();
    for chunk in doc.chunks() {
        let line = match chunk {
            Chunk::Table(rows) => {
                out.push(table_text(&rows, mode));
                continue;
            }
            Chunk::Line(line) => line,
        };
        let text = body(line, backtick_code);
        if line.boxed {
            // Re-pad rather than replay the original spacing: after soft-unwrap the content
            // width has changed, and a box padded to the old width would come out crooked.
            let head = format!("{}{}", line.lead, text);
            let pad = target.saturating_sub(head.width()) + 1;
            out.push(format!("{head}{}{}", " ".repeat(pad), line.tail.trim_start()));
        } else {
            out.push(format!("{}{}{}", line.lead, text, line.tail));
        }
    }
    out.join("\n")
}

fn bidi_safe(doc: &Doc) -> String {
    let target = box_body_width(doc, false);
    let mut lines: Vec<String> = Vec::new();
    for chunk in doc.chunks() {
        let line = match chunk {
            Chunk::Table(rows) => {
                lines.push(table_text(&rows, CopyMode::BidiSafe));
                continue;
            }
            Chunk::Line(line) => line,
        };
        let mut out = String::new();
        if !line.lead.is_empty() {
            out.push(LRI);
            out.push_str(&line.lead);
            out.push(PDI);
        }
        if !line.segments.is_empty() {
            out.push(if line.dir == Dir::Rtl { RLI } else { LRI });
            for seg in &line.segments {
                match seg {
                    Segment::Text(t) => out.push_str(t),
                    Segment::Code(t) => {
                        out.push(LRI);
                        out.push_str(t);
                        out.push(PDI);
                    }
                }
            }
            out.push(PDI);
        }
        if !line.tail.is_empty() {
            // Pad exactly as plain mode does, so a box pasted into Slack or a GitHub
            // comment keeps its right border in a straight line.
            if line.boxed {
                let head = format!("{}{}", line.lead, body(line, false));
                let pad = target.saturating_sub(head.width()) + 1;
                out.extend(std::iter::repeat_n(' ', pad));
            }
            out.push(LRI);
            out.push_str(line.tail.trim_start());
            out.push(PDI);
        }
        lines.push(out);
    }
    lines.join("\n")
}

/// Serialise a document for the clipboard.
pub fn render(doc: &Doc, mode: CopyMode) -> String {
    match mode {
        CopyMode::Plain => plainish(doc, CopyMode::Plain),
        CopyMode::Markdown => plainish(doc, CopyMode::Markdown),
        CopyMode::BidiSafe => bidi_safe(doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{process, CaptureSource, Options};

    fn doc(input: &str) -> Doc {
        process(input, &Options::default(), CaptureSource::Selection)
    }

    #[test]
    fn plain_round_trips_simple_text() {
        let d = doc("سلام دنیا\nhello world");
        assert_eq!(render(&d, CopyMode::Plain), "سلام دنیا\nhello world");
    }

    #[test]
    fn plain_rebuilds_a_square_box() {
        let d = doc("│ کوتاه │\n│ یک خط بلندتر │");
        let out = render(&d, CopyMode::Plain);
        let widths: Vec<usize> = out.lines().map(|l| l.width()).collect();
        assert_eq!(widths[0], widths[1], "box rows must align:\n{out}");
    }

    #[test]
    fn markdown_refences_code_runs() {
        let d = doc("فایل src/main.rs را ببین");
        assert_eq!(render(&d, CopyMode::Markdown), "فایل `src/main.rs` را ببین");
    }

    #[test]
    fn bidi_safe_isolates_code_and_line() {
        let out = render(&doc("فایل src/main.rs را ببین"), CopyMode::BidiSafe);
        assert!(out.starts_with(RLI), "RTL line must open with an RTL isolate");
        assert!(out.ends_with(PDI));
        assert!(out.contains(&format!("{LRI}src/main.rs{PDI}")));
    }

    #[test]
    fn bidi_safe_strips_to_the_same_visible_text() {
        let source = "فایل src/main.rs را ببین";
        let out = render(&doc(source), CopyMode::BidiSafe);
        let visible: String = out.chars().filter(|c| !matches!(*c, LRI | RLI | PDI)).collect();
        assert_eq!(visible, source);
    }

    #[test]
    fn ltr_line_opens_with_an_ltr_isolate() {
        let out = render(&doc("hello world"), CopyMode::BidiSafe);
        assert!(out.starts_with(LRI));
    }

    #[test]
    fn empty_doc_is_empty_string() {
        let d = Doc { lines: vec![], source: CaptureSource::Empty, diff_mode: false };
        assert_eq!(render(&d, CopyMode::Plain), "");
        assert_eq!(render(&d, CopyMode::BidiSafe), "");
    }
}
