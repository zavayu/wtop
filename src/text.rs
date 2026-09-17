use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const LEADING_INDICATOR: char = '‹';
const TRAILING_INDICATOR: char = '›';

/// Measures terminal display cells, saturating at the largest offset wtop
/// stores. Grapheme clusters are used for slicing so byte offsets never split
/// a user-visible character.
pub fn display_width(text: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX)
}

/// Returns the greatest valid display-cell offset for a viewport.
pub fn maximum_scroll_offset(text: &str, viewport_cells: u16) -> u16 {
    let total_width = display_width(text);
    if viewport_cells == 0 || total_width <= viewport_cells {
        return 0;
    }

    // Once scrolled, the leading indicator occupies one cell. Choose a
    // grapheme boundary from which the remaining text fits beside it.
    let final_content_cells = viewport_cells.saturating_sub(1).max(1);
    let mut consumed: u16 = 0;
    for grapheme in text.graphemes(true) {
        consumed = consumed.saturating_add(display_width(grapheme));
        if total_width.saturating_sub(consumed) <= final_content_cells {
            return consumed;
        }
    }

    0
}

/// Produces one terminal-cell viewport over text, with indicators when text
/// exists on either side of the visible content.
pub fn scroll_text(text: &str, requested_offset: u16, viewport_cells: u16) -> String {
    if viewport_cells == 0 {
        return String::new();
    }

    let maximum_offset = maximum_scroll_offset(text, viewport_cells);
    let requested_offset = requested_offset.min(maximum_offset);
    let (visible_text, actual_offset) = text_after_offset(text, requested_offset);
    let has_leading_content = actual_offset > 0;
    let leading_width = u16::from(has_leading_content);
    let remaining_width = display_width(visible_text);
    let content_budget = if remaining_width + leading_width <= viewport_cells {
        viewport_cells - leading_width
    } else {
        viewport_cells.saturating_sub(leading_width + 1)
    };
    let content = truncate_to_width(visible_text, content_budget);
    let has_trailing_content = display_width(&content) < remaining_width;

    let mut rendered = String::new();
    if has_leading_content {
        rendered.push(LEADING_INDICATOR);
    }
    rendered.push_str(&content);
    if has_trailing_content {
        rendered.push(TRAILING_INDICATOR);
    }
    rendered
}

fn text_after_offset(text: &str, offset: u16) -> (&str, u16) {
    let mut consumed: u16 = 0;
    for (byte_index, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = display_width(grapheme);
        if consumed.saturating_add(grapheme_width) > offset {
            return (&text[byte_index..], consumed);
        }
        consumed = consumed.saturating_add(grapheme_width);
    }

    ("", consumed)
}

fn truncate_to_width(text: &str, width: u16) -> String {
    let mut rendered = String::new();
    let mut used_width: u16 = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = display_width(grapheme);
        if used_width.saturating_add(grapheme_width) > width {
            break;
        }
        rendered.push_str(grapheme);
        used_width = used_width.saturating_add(grapheme_width);
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{display_width, maximum_scroll_offset, scroll_text};

    #[test]
    fn display_width_uses_terminal_cells_not_utf8_bytes() {
        assert_eq!(display_width("A界"), 3);
        assert_eq!(display_width("e\u{301}"), 1);
    }

    #[test]
    fn scroll_text_marks_content_outside_the_viewport() {
        assert_eq!(scroll_text("abcdef", 0, 4), "abc›");
        assert_eq!(scroll_text("abcdef", 2, 4), "‹cd›");
        assert_eq!(scroll_text("abcdef", 3, 4), "‹def");
    }

    #[test]
    fn scrolling_never_splits_a_wide_grapheme() {
        assert_eq!(maximum_scroll_offset("A界BC", 4), 3);
        assert_eq!(scroll_text("A界BC", 2, 4), "‹界›");
        assert_eq!(scroll_text("A界BC", 3, 4), "‹BC");
    }
}
