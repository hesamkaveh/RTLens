//! Separate a line's structural frame from its directional content.
//!
//! This is the fix for the box-drawing failure. In `│ متن فارسی │` the pipes are neutral
//! characters, so the bidi algorithm sweeps them into the RTL run started by the first
//! Persian letter and the frame lands in the wrong place. Terminals cannot avoid this,
//! because they resolve each visual line in isolation with no idea a frame exists.
//!
//! Instead of fighting the algorithm we take the frame out of its reach: `lead` and `tail`
//! are laid out physically and only the text between them gets a resolved direction.

/// Characters that carry structure rather than meaning when they sit at a line's edge.
fn is_structural(c: char) -> bool {
    matches!(c,
        '\u{2190}'..='\u{21FF}'   // arrows
        | '\u{2300}'..='\u{23FF}' // misc technical: ⏺ ⎿ , used heavily by CLI agents
        | '\u{2500}'..='\u{257F}' // box drawing
        | '\u{2580}'..='\u{259F}' // block elements
        | '\u{25A0}'..='\u{25FF}' // geometric shapes: ● ○ ◆ ◦
        | '\u{2713}'..='\u{2718}' // check marks and crosses
        | '\u{276E}'..='\u{2771}' // heavy angle brackets: ❯
        | '\u{2022}' | '\u{2023}' | '\u{2043}' // bullets
        | '\u{00B7}' | '\u{2027}' // middle dots
        | '|'
    )
}

/// Length in `char`s of a list or prompt marker starting at `i`, if there is one.
///
/// These are context-dependent: a bare `-` is a bullet only when a space follows it,
/// otherwise it is a minus sign or part of a word.
fn marker_len(chars: &[char], i: usize) -> Option<usize> {
    let next_is_space = |k: usize| chars.get(k).is_some_and(|c| c.is_whitespace());
    match chars[i] {
        '-' | '*' | '+' | '$' if next_is_space(i + 1) => Some(1),
        '>' if next_is_space(i + 1) || chars.get(i + 1) == Some(&'>') => Some(1),
        c if c.is_ascii_digit() => {
            let mut k = i;
            while chars.get(k).is_some_and(|c| c.is_ascii_digit()) {
                k += 1;
            }
            // "1." or "2)" followed by a space is an ordered list marker.
            match chars.get(k) {
                Some('.') | Some(')') if next_is_space(k + 1) => Some(k - i + 1),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A line decomposed into frame and content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    pub lead: String,
    pub content: String,
    pub tail: String,
    /// True when structure appears on both sides, i.e. this is a row inside a box.
    pub boxed: bool,
}

/// Decompose one line. Never fails; a line with no structure yields empty `lead`/`tail`.
pub fn split(line: &str) -> Split {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();

    // Walk in from the left through whitespace, structural glyphs and list markers. The
    // trailing whitespace after the last marker is part of the lead, so content starts
    // exactly at the first real character.
    let mut i = 0usize;
    loop {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        if is_structural(chars[i]) {
            i += 1;
        } else if let Some(len) = marker_len(&chars, i) {
            i += len;
        } else {
            break;
        }
    }
    let lead_end = i;

    // Walk in from the right the same way, but without markers: those are start-only.
    let mut j = n;
    while j > lead_end {
        let c = chars[j - 1];
        if c.is_whitespace() || is_structural(c) {
            j -= 1;
        } else {
            break;
        }
    }
    let tail_start = j;

    let has_structure = |slice: &[char]| slice.iter().any(|c| !c.is_whitespace());
    let lead_structured = has_structure(&chars[..lead_end]);
    let tail_structured = tail_start < n && has_structure(&chars[tail_start..]);
    let boxed = lead_structured && tail_structured && tail_start > lead_end;

    let take = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();

    if boxed {
        Split {
            lead: take(0..lead_end),
            // Interior padding is dropped; CSS handles alignment inside the frame, so a
            // right-aligned RTL row and a left-aligned LTR row both sit flush.
            content: take(lead_end..tail_start).trim_end().to_string(),
            tail: take(tail_start..n),
            boxed: true,
        }
    } else {
        Split {
            lead: take(0..lead_end),
            content: take(lead_end..n).trim_end().to_string(),
            tail: String::new(),
            boxed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(line: &str) -> Split {
        split(line)
    }

    #[test]
    fn boxed_persian_row_keeps_frame_out_of_the_content() {
        let r = s("│ متن فارسی │");
        assert_eq!(r.lead, "│ ");
        assert_eq!(r.content, "متن فارسی");
        assert_eq!(r.tail, " │");
        assert!(r.boxed);
    }

    #[test]
    fn pure_border_line_is_all_lead() {
        let r = s("╭──────────╮");
        assert_eq!(r.lead, "╭──────────╮");
        assert_eq!(r.content, "");
        assert!(!r.boxed);
    }

    #[test]
    fn plain_text_has_no_frame() {
        let r = s("hello world");
        assert_eq!(r.lead, "");
        assert_eq!(r.content, "hello world");
        assert_eq!(r.tail, "");
        assert!(!r.boxed);
    }

    #[test]
    fn indentation_becomes_lead() {
        let r = s("    سلام");
        assert_eq!(r.lead, "    ");
        assert_eq!(r.content, "سلام");
    }

    #[test]
    fn bullet_and_ordered_markers() {
        assert_eq!(s("- یک مورد").lead, "- ");
        assert_eq!(s("• یک مورد").lead, "• ");
        assert_eq!(s("1. اول").lead, "1. ");
        assert_eq!(s("12) دوازدهم").lead, "12) ");
    }

    #[test]
    fn agent_glyph_gutter() {
        let r = s("⏺ در حال اجرا");
        assert_eq!(r.lead, "⏺ ");
        assert_eq!(r.content, "در حال اجرا");
        let r = s("  ⎿  خروجی");
        assert_eq!(r.lead, "  ⎿  ");
        assert_eq!(r.content, "خروجی");
    }

    #[test]
    fn minus_without_space_is_not_a_marker() {
        let r = s("-42 degrees");
        assert_eq!(r.lead, "");
        assert_eq!(r.content, "-42 degrees");
    }

    #[test]
    fn hyphenated_word_is_not_a_marker() {
        assert_eq!(s("e-mail address").lead, "");
    }

    #[test]
    fn prompt_marker_is_gutter() {
        let r = s("$ npm install");
        assert_eq!(r.lead, "$ ");
        assert_eq!(r.content, "npm install");
    }

    #[test]
    fn trailing_whitespace_alone_is_not_a_tail() {
        let r = s("hello   ");
        assert!(!r.boxed);
        assert_eq!(r.tail, "");
        assert_eq!(r.content, "hello");
    }

    #[test]
    fn empty_line() {
        let r = s("");
        assert_eq!(r.lead, "");
        assert_eq!(r.content, "");
        assert!(!r.boxed);
    }

    #[test]
    fn box_row_with_mixed_content() {
        let r = s("│ فایل src/main.rs را ببین   │");
        assert_eq!(r.lead, "│ ");
        assert_eq!(r.content, "فایل src/main.rs را ببین");
        assert_eq!(r.tail, "   │");
        assert!(r.boxed);
    }
}
