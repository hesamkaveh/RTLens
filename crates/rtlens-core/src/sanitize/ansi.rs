//! Turn raw terminal bytes into clean text, without losing the glyphs that carry structure.
//!
//! Everything here is deliberately lossy in one direction only: escape sequences, cursor
//! games and stray bidi controls go away, while box drawing, bullets and whitespace survive
//! untouched because [`crate::bidi::gutter`] depends on them to find a line's frame.

/// Explicit bidi controls injected upstream. They fight the layout we are about to compute,
/// so they always go — except ZWNJ/ZWJ, which Persian orthography and emoji actually need.
fn is_stray_bidi_control(c: char) -> bool {
    matches!(c,
        '\u{200E}' | '\u{200F}'              // LRM, RLM
        | '\u{202A}'..='\u{202E}'            // LRE, RLE, PDF, LRO, RLO
        | '\u{2066}'..='\u{2069}'            // LRI, RLI, FSI, PDI
        | '\u{061C}'                         // ARABIC LETTER MARK
    )
}

/// Replay a single line's carriage returns and backspaces the way a terminal would, so a
/// progress bar collapses to its final frame instead of a wall of repeated garbage.
fn replay_cursor(line: &str) -> String {
    if !line.contains(['\r', '\u{8}']) {
        return line.to_string();
    }
    let mut cells: Vec<char> = Vec::with_capacity(line.len());
    let mut col = 0usize;
    for ch in line.chars() {
        match ch {
            '\r' => col = 0,
            '\u{8}' => col = col.saturating_sub(1),
            c => {
                if col < cells.len() {
                    cells[col] = c;
                } else {
                    cells.push(c);
                }
                col += 1;
            }
        }
    }
    cells.into_iter().collect()
}

/// Expand tabs to the next tab stop. Runs after cursor replay so columns are already settled.
fn expand_tabs(line: &str, tab_width: usize) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let width = tab_width.max(1);
    let mut out = String::with_capacity(line.len());
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let pad = width - (col % width);
            out.extend(std::iter::repeat_n(' ', pad));
            col += pad;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// Carriage return, tab and backspace are the three control characters we still need
/// *after* escape-sequence removal — but the VTE parser inside `strip_ansi_escapes`
/// consumes every C0 control, including those. They are swapped for private-use
/// sentinels across that call and restored immediately afterwards.
const SENTINELS: [(char, char); 3] = [('\r', '\u{E000}'), ('\t', '\u{E001}'), ('\u{8}', '\u{E002}')];

fn protect(s: &str) -> String {
    let mut out = s.to_string();
    for (real, holder) in SENTINELS {
        if out.contains(real) {
            out = out.replace(real, &holder.to_string());
        }
    }
    out
}

fn unprotect(s: &str) -> String {
    let mut out = s.to_string();
    for (real, holder) in SENTINELS {
        if out.contains(holder) {
            out = out.replace(holder, &real.to_string());
        }
    }
    out
}

/// Full cleanup pass: ANSI/OSC removal, cursor replay, tab expansion, control and BOM
/// scrubbing. Box drawing (U+2500..U+259F), bullets and spacing are preserved verbatim.
pub fn clean(input: &str, tab_width: usize) -> String {
    // `strip-ansi-escapes` runs a real VTE parser, so CSI/SGR/OSC (including OSC 8
    // hyperlinks, whose label survives) and bracketed-paste markers all resolve here.
    let stripped = strip_ansi_escapes::strip(protect(input));
    let stripped = String::from_utf8_lossy(&stripped);
    let restored = unprotect(&stripped);

    let normalized = restored.replace("\r\n", "\n");

    let mut out = String::with_capacity(normalized.len());
    for (i, raw_line) in normalized.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line = replay_cursor(raw_line);
        let line = expand_tabs(&line, tab_width);
        // A terminal can pad an otherwise blank screen row to its full column width.
        // Keeping that padding creates a non-shrinking gutter in the HUD and therefore a
        // horizontal scrollbar. Collapse only content-free rows; indentation on lines
        // containing text remains byte-for-byte intact.
        if line.chars().all(char::is_whitespace) {
            continue;
        }
        for ch in line.chars() {
            if is_stray_bidi_control(ch) || ch == '\u{FEFF}' {
                continue;
            }
            // Anything still a control character at this point is noise.
            if ch.is_control() {
                continue;
            }
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sgr_colour() {
        assert_eq!(clean("\x1b[31mred\x1b[0m text", 4), "red text");
    }

    #[test]
    fn keeps_osc8_hyperlink_label() {
        let input = "see \x1b]8;;https://example.com\x07the docs\x1b]8;;\x07 now";
        assert_eq!(clean(input, 4), "see the docs now");
    }

    #[test]
    fn drops_bracketed_paste_markers() {
        assert_eq!(clean("\x1b[200~pasted\x1b[201~", 4), "pasted");
    }

    #[test]
    fn collapses_progress_bar_to_final_frame() {
        let input = "  10%\r  50%\r 100% done";
        assert_eq!(clean(input, 4), " 100% done");
    }

    #[test]
    fn applies_backspace_overstrike() {
        assert_eq!(clean("X\u{8}X", 4), "X");
    }

    #[test]
    fn preserves_box_drawing_and_bullets() {
        let input = "╭─────╮\n│ • hi │\n╰─────╯";
        assert_eq!(clean(input, 4), input);
    }

    #[test]
    fn expands_tabs_to_stops() {
        assert_eq!(clean("ab\tc", 4), "ab  c");
        assert_eq!(clean("a\tb", 4), "a   b");
    }

    #[test]
    fn removes_stray_bidi_controls_but_keeps_zwnj() {
        // U+200F RLM must go; U+200C ZWNJ is load-bearing in Persian ("می‌رود").
        let input = "\u{200F}می\u{200C}رود";
        assert_eq!(clean(input, 4), "می\u{200C}رود");
    }

    #[test]
    fn preserves_trailing_and_leading_whitespace() {
        assert_eq!(clean("   indented   ", 4), "   indented   ");
    }

    #[test]
    fn collapses_whitespace_only_lines_without_trimming_indentation() {
        let input =
            "question\n                                                            \n    indented answer";
        assert_eq!(clean(input, 4), "question\n\n    indented answer");
    }
}
