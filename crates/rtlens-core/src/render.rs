//! Render a [`Doc`] to HTML.
//!
//! The shipping HUD builds the same DOM in TypeScript; this exists so the engine can be
//! inspected visually offline, and so snapshot tests assert on real markup rather than on
//! a debug format that could drift from what users see.

use crate::model::{Chunk, Doc, Line, Segment, TableRow};

/// The canonical layout rules, shared verbatim with the frontend.
pub const LAYOUT_CSS: &str = include_str!("../../../src/styles/layout.css");

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn segments_html(segments: &[Segment]) -> String {
    let mut html = String::new();
    for seg in segments {
        match seg {
            Segment::Text(t) => html.push_str(&escape(t)),
            Segment::Code(t) => {
                html.push_str(&format!("<span class=\"rtl-code\" dir=\"ltr\">{}</span>", escape(t)))
            }
        }
    }
    html
}

/// A recovered table, as a grid.
///
/// The columns are `auto`-sized by the browser rather than by the widths the terminal
/// painted: those widths were chosen for a monospace grid, and the HUD is proportional.
/// What has to survive is the column *structure*, not its former pixel geometry.
fn table_html(rows: &[&TableRow]) -> String {
    let first = rows[0];
    let mut classes = String::from("rtl-table");
    if first.framed {
        classes.push_str(" rtl-table--framed");
    }
    let mut html = format!(
        "<div class=\"{}\" dir=\"{}\" style=\"--rtl-cols:{}\">",
        classes,
        first.dir.as_attr(),
        first.columns
    );
    for row in rows {
        html.push_str(if row.head {
            "<div class=\"rtl-row rtl-row--head\">"
        } else {
            "<div class=\"rtl-row\">"
        });
        for cell in &row.cells {
            html.push_str(&format!(
                "<div class=\"rtl-cell rtl-cell--{}\" dir=\"{}\">{}</div>",
                cell.align.as_attr(),
                cell.dir.as_attr(),
                segments_html(&cell.segments)
            ));
        }
        html.push_str("</div>");
    }
    html.push_str("</div>");
    html
}

fn line_html(line: &Line) -> String {
    let mut classes = vec!["rtl-line".to_string()];
    if line.code {
        classes.push("rtl-line--code".into());
    }
    if line.is_blank() {
        classes.push("rtl-line--blank".into());
    }
    // A frame stays physical; a marker or indent follows the reading direction.
    if !line.boxed && line.dir == crate::Dir::Rtl && !line.lead.is_empty() {
        classes.push("rtl-line--rtl".into());
    }

    let mut html = format!("<div class=\"{}\">", classes.join(" "));
    if !line.lead.is_empty() {
        html.push_str(&format!("<span class=\"rtl-gutter\">{}</span>", escape(&line.lead)));
    }
    html.push_str(&format!("<span class=\"rtl-content\" dir=\"{}\">", line.dir.as_attr()));
    html.push_str(&segments_html(&line.segments));
    html.push_str("</span>");
    if !line.tail.is_empty() {
        html.push_str(&format!("<span class=\"rtl-gutter rtl-gutter--tail\">{}</span>", escape(&line.tail)));
    }
    html.push_str("</div>");
    html
}

/// The document body, without any wrapper chrome.
pub fn fragment(doc: &Doc) -> String {
    let parts: Vec<String> = doc
        .chunks()
        .into_iter()
        .map(|chunk| match chunk {
            Chunk::Line(line) => line_html(line),
            Chunk::Table(rows) => table_html(&rows),
        })
        .collect();
    format!("<div class=\"rtl-doc\">{}</div>", parts.join("\n"))
}

/// A self-contained page for offline inspection.
pub fn standalone(doc: &Doc, title: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n<title>{}</title>\n\
         <style>\nbody {{ margin: 0; padding: 24px; background: #14161a; color: #e6e6e6; }}\n\
         {}\n</style></head>\n<body>\n{}\n</body></html>\n",
        escape(title),
        LAYOUT_CSS,
        fragment(doc)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{process, CaptureSource, Options};

    fn html(input: &str) -> String {
        fragment(&process(input, &Options::default(), CaptureSource::Selection))
    }

    #[test]
    fn sets_dir_per_line() {
        let out = html("سلام\nhello");
        assert!(out.contains(r#"<span class="rtl-content" dir="rtl">"#));
        assert!(out.contains(r#"<span class="rtl-content" dir="ltr">"#));
    }

    #[test]
    fn code_runs_are_ltr_isolated() {
        let out = html("فایل /src/main.rs را ببین");
        assert!(out.contains(r#"<span class="rtl-code" dir="ltr">/src/main.rs</span>"#));
    }

    #[test]
    fn box_frame_lands_in_gutters_not_content() {
        let out = html("│ سلام │");
        assert!(out.contains(r#"<span class="rtl-gutter">│ </span>"#));
        assert!(out.contains("rtl-gutter--tail"));
        // The frame must never end up inside the directional content span.
        let content = out.split(r#"class="rtl-content""#).nth(1).unwrap();
        let content = content.split("</span>").next().unwrap();
        assert!(!content.contains('│'));
    }

    #[test]
    fn rtl_bullet_line_flips_but_boxed_line_does_not() {
        assert!(html("- سلام").contains("rtl-line--rtl"));
        assert!(!html("│ سلام │").contains("rtl-line--rtl"));
    }

    #[test]
    fn escapes_markup_in_content() {
        let out = html("<script>alert(1)</script>");
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn standalone_page_embeds_the_shared_css() {
        let d = process("سلام", &Options::default(), CaptureSource::Selection);
        let page = standalone(&d, "t");
        assert!(page.contains("unicode-bidi: isolate"));
        assert!(page.starts_with("<!doctype html>"));
    }
}
