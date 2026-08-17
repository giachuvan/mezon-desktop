use std::any::Any;
use std::ops::Range;
use std::rc::Rc;

use gpui::SharedString;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_UNDO_HISTORY: usize = 256;

fn word_bound_segments(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    text.split_word_bound_indices()
        .map(|(start, segment)| (start, start + segment.len()))
}

fn segment_is_whitespace(text: &str, start: usize, end: usize) -> bool {
    text[start..end].chars().all(char::is_whitespace)
}

fn segment_is_word(text: &str, start: usize, end: usize) -> bool {
    text[start..end]
        .chars()
        .any(|c| c.is_alphanumeric() || c == '_')
}

pub fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == 0 {
        return 0;
    }
    let mut start = 0;
    for (seg_start, seg_end) in word_bound_segments(text) {
        if seg_start >= offset {
            break;
        }
        if !segment_is_whitespace(text, seg_start, seg_end) {
            start = seg_start;
        }
    }
    start
}

pub fn next_word_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == text.len() {
        return text.len();
    }
    for (seg_start, seg_end) in word_bound_segments(text) {
        if seg_end <= offset {
            continue;
        }
        if !segment_is_whitespace(text, seg_start, seg_end) {
            return seg_end;
        }
    }
    text.len()
}

pub fn line_start(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

pub fn line_end(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    text[offset..]
        .find('\n')
        .map(|i| offset + i)
        .unwrap_or(text.len())
}

pub fn home_target(text: &str, offset: usize) -> usize {
    let start = line_start(text, offset);
    let end = line_end(text, offset);
    let indent_end = text[start..end]
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, _)| start + i)
        .unwrap_or(start);
    if offset == indent_end {
        start
    } else {
        indent_end
    }
}

pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let mut best: Option<(usize, usize, u8)> = None;
    for (start, end) in word_bound_segments(text) {
        if start > offset || end < offset || start == end {
            continue;
        }
        let rank = if segment_is_word(text, start, end) {
            2
        } else if !segment_is_whitespace(text, start, end) {
            1
        } else {
            0
        };
        match best {
            Some((_, _, best_rank)) if rank < best_rank => {}
            Some((best_start, _, best_rank)) if rank == best_rank && start > best_start => {}
            _ => best = Some((start, end, rank)),
        }
    }
    best.map(|(start, end, _)| start..end)
        .unwrap_or(offset..offset)
}

pub fn line_range_at(text: &str, offset: usize) -> Range<usize> {
    line_start(text, offset)..line_end(text, offset)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelectGranularity {
    #[default]
    Character,
    Word,
    Line,
}

pub fn granularity_for_click(click_count: usize) -> SelectGranularity {
    match click_count {
        0 | 1 => SelectGranularity::Character,
        2 => SelectGranularity::Word,
        _ => SelectGranularity::Line,
    }
}

pub fn range_for_granularity(
    text: &str,
    offset: usize,
    granularity: SelectGranularity,
    multi_line: bool,
) -> Range<usize> {
    match granularity {
        SelectGranularity::Character => offset..offset,
        SelectGranularity::Word => word_range_at(text, offset),
        SelectGranularity::Line if multi_line => line_range_at(text, offset),
        SelectGranularity::Line => 0..text.len(),
    }
}

pub fn extend_range_for_granularity(
    text: &str,
    anchor: &Range<usize>,
    offset: usize,
    granularity: SelectGranularity,
    multi_line: bool,
) -> (Range<usize>, bool) {
    let unit = range_for_granularity(text, offset, granularity, multi_line);
    if unit.start < anchor.start {
        (unit.start..anchor.end, true)
    } else {
        (anchor.start..unit.end.max(anchor.end), false)
    }
}

#[derive(Clone)]
pub struct HistoryEntry {
    pub content: SharedString,
    pub selected_range: Range<usize>,
    pub selection_reversed: bool,
    pub payload: Option<Rc<dyn Any>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Insert,
    Delete,
    Other,
}

pub fn should_coalesce(last: Option<EditKind>, kind: EditKind) -> bool {
    matches!(kind, EditKind::Insert | EditKind::Delete) && last == Some(kind)
}

pub fn floor_char_boundary(text: &str, index: usize) -> usize {
    let index = index.min(text.len());
    if text.is_char_boundary(index) {
        return index;
    }
    let mut i = index;
    while i > 0 {
        i -= 1;
        if text.is_char_boundary(i) {
            return i;
        }
    }
    0
}

pub fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let index = index.min(text.len());
    if text.is_char_boundary(index) {
        return index;
    }
    let mut i = index + 1;
    while i < text.len() {
        if text.is_char_boundary(i) {
            return i;
        }
        i += 1;
    }
    text.len()
}

pub fn surrounding_delete_range(
    text: &str,
    selected_range: &Range<usize>,
    marked_range: Option<&Range<usize>>,
    selection_reversed: bool,
    before_len: usize,
    after_len: usize,
) -> Range<usize> {
    let (left, right) = if let Some(marked) = marked_range {
        (marked.start.min(text.len()), marked.end.min(text.len()))
    } else if selected_range.start != selected_range.end {
        (
            selected_range.start.min(text.len()),
            selected_range.end.min(text.len()),
        )
    } else {
        let caret = if selection_reversed {
            selected_range.start
        } else {
            selected_range.end
        }
        .min(text.len());
        (caret, caret)
    };
    let start = floor_char_boundary(text, left.saturating_sub(before_len));
    let end = ceil_char_boundary(text, right.saturating_add(after_len));
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_word_boundary_skips_trailing_space_then_word() {
        let text = "hello world";
        assert_eq!(previous_word_boundary(text, 11), 6);
        assert_eq!(previous_word_boundary(text, 6), 0);
        assert_eq!(previous_word_boundary(text, 8), 6);
        assert_eq!(previous_word_boundary(text, 0), 0);
    }

    #[test]
    fn next_word_boundary_skips_leading_space_then_word() {
        let text = "hello world";
        assert_eq!(next_word_boundary(text, 0), 5);
        assert_eq!(next_word_boundary(text, 5), 11);
        assert_eq!(next_word_boundary(text, 2), 5);
        assert_eq!(next_word_boundary(text, 11), 11);
    }

    #[test]
    fn word_boundary_treats_hyphen_as_its_own_run() {
        let text = "foo-bar baz";
        assert_eq!(next_word_boundary(text, 0), 3);
        assert_eq!(next_word_boundary(text, 3), 4);
        assert_eq!(previous_word_boundary(text, 7), 4);
        assert_eq!(previous_word_boundary(text, 4), 3);
    }

    #[test]
    fn word_boundary_keeps_a_mid_letter_dot_together() {
        let text = "foo.bar baz";
        assert_eq!(next_word_boundary(text, 0), 7);
        assert_eq!(previous_word_boundary(text, 7), 0);
    }

    #[test]
    fn word_boundary_respects_utf8_boundaries() {
        let text = "chào bạn";
        assert_eq!("chào".len(), 5);
        assert_eq!(next_word_boundary(text, 0), 5);
        assert_eq!(previous_word_boundary(text, text.len()), 6);
    }

    #[test]
    fn line_start_and_end_bracket_the_current_line() {
        let text = "ab\ncde\nf";
        assert_eq!(line_start(text, 5), 3);
        assert_eq!(line_end(text, 5), 6);
        assert_eq!(line_start(text, 0), 0);
        assert_eq!(line_end(text, 0), 2);
        assert_eq!(line_start(text, 8), 7);
        assert_eq!(line_end(text, 8), 8);
    }

    #[test]
    fn home_target_toggles_between_indent_and_column_zero() {
        let text = "    code";
        assert_eq!(home_target(text, 8), 4);
        assert_eq!(home_target(text, 4), 0);
        assert_eq!(home_target(text, 0), 4);
    }

    #[test]
    fn home_target_on_unindented_line_is_line_start() {
        let text = "one\ntwo";
        assert_eq!(home_target(text, 6), 4);
        assert_eq!(home_target(text, 4), 4);
    }

    #[test]
    fn word_range_covers_the_word_under_the_caret() {
        let text = "foo bar baz";
        assert_eq!(word_range_at(text, 5), 4..7);
        assert_eq!(word_range_at(text, 4), 4..7);
        assert_eq!(word_range_at(text, 7), 4..7);
    }

    #[test]
    fn word_range_prefers_the_word_before_a_trailing_space() {
        let text = "foo bar";
        assert_eq!(word_range_at(text, 3), 0..3);
        assert_eq!(word_range_at(text, 7), 4..7);
    }

    #[test]
    fn word_range_treats_hyphen_as_its_own_run() {
        let text = "foo-bar";
        assert_eq!(word_range_at(text, 0), 0..3);
        assert_eq!(word_range_at(text, 4), 4..7);
    }

    #[test]
    fn word_range_at_a_hyphen_boundary_prefers_the_word() {
        let text = "foo-bar";
        assert_eq!(word_range_at(text, 3), 0..3);
        assert_eq!(word_range_at(text, 4), 4..7);
    }

    #[test]
    fn word_range_keeps_a_mid_letter_dot_together() {
        let text = "foo.bar";
        assert_eq!(word_range_at(text, 0), 0..7);
        assert_eq!(word_range_at(text, 3), 0..7);
        assert_eq!(word_range_at(text, 4), 0..7);
    }

    #[test]
    fn word_range_respects_utf8_boundaries() {
        let text = "chào bạn";
        assert_eq!(word_range_at(text, 2), 0..5);
        assert_eq!(word_range_at(text, 7), 6..text.len());
    }

    #[test]
    fn word_range_keeps_a_uax29_contraction_together() {
        let text = "don't stop";
        assert_eq!(word_range_at(text, 2), 0..5);
        assert_eq!(next_word_boundary(text, 0), 5);
        assert_eq!(previous_word_boundary(text, 5), 0);
        assert_eq!(previous_word_boundary(text, text.len()), 6);
    }

    #[test]
    fn word_range_treats_vietnamese_letters_as_one_word() {
        let text = "tiếng Việt";
        assert_eq!(word_range_at(text, 2), 0.."tiếng".len());
        assert_eq!(next_word_boundary(text, 0), "tiếng".len());
        assert_eq!(previous_word_boundary(text, text.len()), "tiếng ".len());
    }

    #[test]
    fn word_range_on_empty_text_is_empty() {
        assert_eq!(word_range_at("", 0), 0..0);
        assert_eq!(word_range_at("   ", 1), 0..3);
    }

    #[test]
    fn triple_click_selects_the_line_only_when_multi_line() {
        let text = "one\ntwo";
        let line = SelectGranularity::Line;
        assert_eq!(range_for_granularity(text, 5, line, true), 4..7);
        assert_eq!(range_for_granularity(text, 5, line, false), 0..7);
    }

    #[test]
    fn granularity_maps_click_count_to_word_then_line() {
        assert_eq!(granularity_for_click(1), SelectGranularity::Character);
        assert_eq!(granularity_for_click(2), SelectGranularity::Word);
        assert_eq!(granularity_for_click(3), SelectGranularity::Line);
        assert_eq!(granularity_for_click(4), SelectGranularity::Line);
    }

    #[test]
    fn dragging_by_word_keeps_the_anchor_word_whole() {
        let text = "foo bar baz";
        let anchor = 4..7;
        let word = SelectGranularity::Word;

        let (forward, reversed) = extend_range_for_granularity(text, &anchor, 9, word, false);
        assert_eq!(forward, 4..11);
        assert!(!reversed);

        let (backward, reversed) = extend_range_for_granularity(text, &anchor, 1, word, false);
        assert_eq!(backward, 0..7);
        assert!(reversed);
    }

    #[test]
    fn dragging_back_inside_the_anchor_word_does_not_shrink_it() {
        let text = "foo bar baz";
        let anchor = 4..7;
        let (range, reversed) =
            extend_range_for_granularity(text, &anchor, 5, SelectGranularity::Word, false);
        assert_eq!(range, 4..7);
        assert!(!reversed);
    }

    #[test]
    fn surrounding_delete_removes_the_vowel_before_a_telex_tone() {
        assert_eq!(
            surrounding_delete_range("hoa", &(3..3), None, false, 1, 0),
            2..3
        );
    }

    #[test]
    fn surrounding_delete_snaps_mid_character_offsets_to_utf8_boundaries() {
        let text = "á";
        let end = text.len();
        let range = surrounding_delete_range(text, &(end..end), None, false, 1, 0);
        assert!(text.is_char_boundary(range.start));
        assert!(text.is_char_boundary(range.end));
        assert_eq!(&text[range], "á");
    }
}
