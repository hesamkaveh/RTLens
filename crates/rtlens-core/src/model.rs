//! Wire types shared between the Rust engine and the webview renderer.

use serde::{Deserialize, Serialize};

/// Resolved base direction of a single line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Ltr,
    Rtl,
}

impl Dir {
    /// Value for the HTML `dir` attribute.
    pub fn as_attr(self) -> &'static str {
        match self {
            Dir::Ltr => "ltr",
            Dir::Rtl => "rtl",
        }
    }
}

/// A run of line content. `Code` runs are rendered LTR-isolated in a monospace face;
/// `Text` runs inherit the line's base direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "text", rename_all = "lowercase")]
pub enum Segment {
    Text(String),
    Code(String),
}

impl Segment {
    pub fn text(&self) -> &str {
        match self {
            Segment::Text(s) | Segment::Code(s) => s,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text().is_empty()
    }
}

/// How a table's columns were divided in the capture, so a copy can put them back the
/// same way rather than imposing one house style on everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sep {
    /// A vertical rule stood between the cells, e.g. `│ a │ b │`.
    Delimited(char),
    /// The cells were held apart by blank columns.
    Spaced,
}

/// One cell of a table row. Each carries its own direction: that is the entire point of
/// recovering the columns, since a shared direction is what made them swap places.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    /// The cell's own direction, resolved from its own letters. This is what keeps a Latin
    /// cell from being reordered by the Persian one beside it.
    pub dir: Dir,
    /// The direction of the *column* the cell sits in, which decides the edge its text is
    /// aligned to. A column whose header is Persian and whose body is Latin still has to
    /// align as one column, so this is not always the cell's own direction.
    pub align: Dir,
    pub segments: Vec<Segment>,
}

impl Cell {
    pub fn text(&self) -> String {
        self.segments.iter().map(Segment::text).collect()
    }
}

/// The table membership of a [`Line`].
///
/// Table-wide facts are repeated on every row so the wire format stays a flat list of
/// lines: renderers group by `id`, and consecutive rows sharing an `id` are one table.
/// Repetition beats nesting here because every other consumer — copy, measurement, the
/// blank-line check — keeps working on a flat list without knowing tables exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    pub id: u32,
    pub columns: usize,
    /// Reading direction of the table as a whole, which is the order its columns run in.
    /// A Persian table starts at the right, and rebuilding it left-to-right would reverse
    /// the column order the reader is looking for.
    pub dir: Dir,
    /// True for the header row above the first rule.
    pub head: bool,
    /// True when the source drew an outer frame around the table.
    pub framed: bool,
    pub sep: Sep,
    pub cells: Vec<Cell>,
}

/// One source line, split into a structural gutter and directional content.
///
/// `lead` and `tail` hold the box frame / list marker / indent. They are always laid out
/// physically (LTR) so a frame keeps its shape no matter which way the content runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub lead: String,
    pub tail: String,
    /// True when the line has non-blank structure on *both* sides, i.e. a boxed row.
    pub boxed: bool,
    /// True when the line sits inside a fenced code block.
    pub code: bool,
    pub dir: Dir,
    pub segments: Vec<Segment>,
    /// Set when the line is a row of a recovered table; `segments` then holds the same
    /// text flattened, so anything that does not know about tables still reads sensibly.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub table: Option<TableRow>,
}

impl Line {
    /// The line's content with structure stripped, as plain text.
    pub fn content(&self) -> String {
        self.segments.iter().map(Segment::text).collect()
    }

    pub fn is_blank(&self) -> bool {
        self.segments.iter().all(|s| s.text().trim().is_empty())
            && self.lead.trim().is_empty()
            && self.tail.trim().is_empty()
    }
}

/// A document as the pieces a consumer has to handle as a unit.
///
/// Grouping lives here, not in each renderer, because "consecutive rows with the same id
/// are one table" is a fact about the format rather than about any one way of drawing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk<'a> {
    Line(&'a Line),
    Table(Vec<&'a TableRow>),
}

/// Where the captured text came from, so the HUD can be honest about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSource {
    /// A fresh selection, captured via synthetic copy or primary selection.
    Selection,
    /// The clipboard did not change; showing whatever was already on it.
    ClipboardFallback,
    /// Nothing usable to show.
    Empty,
}

/// A fully analysed document, ready to render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub lines: Vec<Line>,
    pub source: CaptureSource,
    /// Set when the input was detected as a unified diff; the renderer keeps everything LTR.
    pub diff_mode: bool,
}

impl Doc {
    /// Walk the document, keeping each table's rows together.
    pub fn chunks(&self) -> Vec<Chunk<'_>> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < self.lines.len() {
            match &self.lines[i].table {
                Some(first) => {
                    let mut rows = vec![first];
                    let mut end = i + 1;
                    while let Some(next) = self.lines.get(end).and_then(|l| l.table.as_ref()) {
                        if next.id != first.id {
                            break;
                        }
                        rows.push(next);
                        end += 1;
                    }
                    out.push(Chunk::Table(rows));
                    i = end;
                }
                None => {
                    out.push(Chunk::Line(&self.lines[i]));
                    i += 1;
                }
            }
        }
        out
    }
}

/// Engine toggles, surfaced in the HUD and persisted in settings.
///
/// `default` is load-bearing, not decoration: without it, adding a field in a later version
/// makes every existing settings file fail to deserialise, which would silently reset the
/// user's shortcut and preferences on upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// Fold Arabic letterforms onto their Persian equivalents.
    pub normalize_persian: bool,
    /// Rejoin lines the terminal hard-wrapped mid-sentence.
    pub soft_unwrap: bool,
    /// Drop the backticks around an inline code span.
    pub strip_backticks: bool,
    pub tab_width: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { normalize_persian: true, soft_unwrap: true, strip_backticks: true, tab_width: 4 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_tolerate_a_settings_file_from_an_older_version() {
        // A file written before a field existed must still load, keeping the other values.
        let older = serde_json::json!({ "softUnwrap": false });
        let opts: Options = serde_json::from_value(older).expect("must not fail");
        assert!(!opts.soft_unwrap);
        assert_eq!(opts.tab_width, Options::default().tab_width);
        assert_eq!(opts.normalize_persian, Options::default().normalize_persian);
    }

    #[test]
    fn options_round_trip() {
        let opts = Options { soft_unwrap: false, tab_width: 8, ..Options::default() };
        let json = serde_json::to_value(opts).unwrap();
        assert_eq!(serde_json::from_value::<Options>(json).unwrap(), opts);
    }
}
