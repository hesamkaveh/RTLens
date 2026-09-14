//! Direction resolution and line structure.

pub mod direction;
pub mod gutter;
pub mod segment;

use crate::model::Dir;

/// A line after structural splitting but before inline segmentation.
///
/// Soft-unwrap works at this stage: it needs the gutter and direction already resolved,
/// but merging lines invalidates segmentation, so segmentation comes last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine {
    pub lead: String,
    pub content: String,
    pub tail: String,
    pub boxed: bool,
    pub in_code: bool,
    pub dir: Dir,
    /// One row of a detected table: the cells as text, plus the table-wide facts needed to
    /// rebuild it. Set before soft-unwrap runs, which is what keeps unwrap from welding a
    /// row to its neighbour.
    pub row: Option<RawRow>,
}

/// A table row before its cells have been given directions and segmented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRow {
    pub id: u32,
    pub columns: usize,
    pub dir: Dir,
    pub head: bool,
    pub framed: bool,
    pub sep: crate::model::Sep,
    pub cells: Vec<String>,
    /// Per-column direction, so a column aligns to one edge for its whole height.
    pub align: Vec<Dir>,
}
