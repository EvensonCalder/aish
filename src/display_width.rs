use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn display_width(text: &str) -> usize {
    if !text.contains('\t') {
        return UnicodeWidthStr::width(text);
    }
    text.split('\t').map(UnicodeWidthStr::width).sum::<usize>()
        + text.chars().filter(|ch| *ch == '\t').count()
}

pub fn truncate_end_with_ellipsis(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let available = width - 3;
    let mut prefix = String::new();
    let mut used = 0;
    for ch in value.chars() {
        let ch_width = char_display_width(ch);
        if used + ch_width > available {
            break;
        }
        prefix.push(ch);
        used += ch_width;
    }
    format!("{prefix}...")
}

pub fn truncate_start_with_ellipsis(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let available = width - 3;
    let mut suffix = Vec::new();
    let mut used = 0;
    for ch in value.chars().rev() {
        let ch_width = char_display_width(ch);
        if used + ch_width > available {
            break;
        }
        suffix.push(ch);
        used += ch_width;
    }
    suffix.reverse();
    format!("...{}", suffix.into_iter().collect::<String>())
}

pub fn visual_line_count(text: &str, width: usize) -> usize {
    visual_position(text, width).0 + 1
}

pub fn visual_position(text: &str, width: usize) -> (usize, u16) {
    let width = width.max(1);
    let mut row = 0usize;
    let mut col = 0usize;

    for ch in text.chars() {
        if ch == '\n' {
            row += 1;
            col = 0;
            continue;
        }

        let ch_width = char_display_width(ch);
        if ch_width == 0 {
            continue;
        }
        if col + ch_width > width {
            row += 1;
            col = 0;
        }
        col += ch_width;
        while col >= width {
            row += 1;
            col -= width;
        }
    }

    (row, col.min(u16::MAX as usize) as u16)
}

fn char_display_width(ch: char) -> usize {
    if ch == '\t' {
        return 1;
    }
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_cjk_as_full_width() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("a中b"), 4);
        assert_eq!(display_width("ＡＢ"), 4);
        assert_eq!(display_width("a\t中"), 4);
    }

    #[test]
    fn truncates_to_display_width() {
        assert_eq!(truncate_end_with_ellipsis("中abcd", 5), "中...");
        assert_eq!(display_width(&truncate_end_with_ellipsis("中abcd", 5)), 5);
        assert_eq!(truncate_start_with_ellipsis("abcd中", 5), "...中");
        assert_eq!(display_width(&truncate_start_with_ellipsis("abcd中", 5)), 5);
    }

    #[test]
    fn visual_position_wraps_full_width_characters() {
        assert_eq!(visual_position("> ab", 4), (1, 0));
        assert_eq!(visual_position("> a中b", 6), (1, 0));
        assert_eq!(visual_position("> a中", 6), (0, 5));
    }

    #[test]
    fn visual_position_wraps_before_full_width_character_that_does_not_fit() {
        assert_eq!(visual_position("abc中", 4), (1, 2));
    }
}
