use std::ops::Range;

use gpui::{AnyElement, ClipboardItem, Context, KeyDownEvent, UTF16Selection, div, prelude::*};
use unicode_segmentation::UnicodeSegmentation;

use crate::{theme, ui};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Selection {
    anchor: usize,
    caret: usize,
}

impl Selection {
    fn range(self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn is_empty(self) -> bool {
        self.anchor == self.caret
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkedText {
    range: Range<usize>,
    original: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TextInput {
    text: String,
    selection: Selection,
    marked: Option<MarkedText>,
}

impl TextInput {
    #[cfg(test)]
    pub(crate) fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            selection: Selection {
                anchor: end,
                caret: end,
            },
            marked: None,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn reset(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.move_to_end();
        self.marked = None;
    }

    pub(crate) fn move_to_end(&mut self) {
        let end = self.text.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
    }

    pub(crate) fn unmark_text(&mut self) {
        self.marked = None;
    }

    pub(crate) fn text_for_range(
        &self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
    ) -> String {
        let range = utf16_range_to_utf8(&self.text, &range_utf16);
        adjusted_range.replace(utf8_range_to_utf16(&self.text, &range));
        self.text[range].to_string()
    }

    pub(crate) fn selected_text_range(&self) -> UTF16Selection {
        UTF16Selection {
            range: utf8_range_to_utf16(&self.text, &self.selection.range()),
            reversed: self.selection.caret < self.selection.anchor,
        }
    }

    pub(crate) fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|marked| utf8_range_to_utf16(&self.text, &marked.range))
    }

    pub(crate) fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
    ) -> bool {
        let range = self.resolve_range(range_utf16);
        let new_text = single_line(new_text);
        let changed = self.text[range.clone()] != new_text;
        self.text.replace_range(range.clone(), &new_text);
        let end = range.start + new_text.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
        self.marked = None;
        changed
    }

    pub(crate) fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) -> bool {
        let range = self.resolve_range(range_utf16);
        let original = self
            .marked
            .as_ref()
            .filter(|marked| marked.range == range)
            .map(|marked| marked.original.clone())
            .unwrap_or_else(|| self.text[range.clone()].to_string());
        let new_text = single_line(new_text);
        let changed = self.text[range.clone()] != new_text;
        self.text.replace_range(range.clone(), &new_text);
        let marked_range = range.start..range.start + new_text.len();
        self.marked = (!new_text.is_empty()).then_some(MarkedText {
            range: marked_range.clone(),
            original,
        });
        self.selection = new_selected_range_utf16
            .map(|selection| {
                let anchor = range.start + utf16_offset_to_utf8(&new_text, selection.start);
                let caret = range.start + utf16_offset_to_utf8(&new_text, selection.end);
                Selection { anchor, caret }
            })
            .unwrap_or_else(|| Selection {
                anchor: marked_range.end,
                caret: marked_range.end,
            });
        changed
    }

    #[cfg(test)]
    pub(crate) fn cancel_marked_text(&mut self) -> bool {
        let Some(marked) = self.marked.take() else {
            return false;
        };
        let changed = self.text[marked.range.clone()] != marked.original;
        self.text
            .replace_range(marked.range.clone(), &marked.original);
        let end = marked.range.start + marked.original.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
        changed
    }

    pub(crate) fn character_index_utf16(&self) -> usize {
        self.text[..self.selection.caret].encode_utf16().count()
    }

    fn resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| utf16_range_to_utf8(&self.text, &range))
            .or_else(|| self.marked.as_ref().map(|marked| marked.range.clone()))
            .unwrap_or_else(|| self.selection.range())
    }

    fn selected_text(&self) -> Option<&str> {
        let range = self.selection.range();
        (!range.is_empty()).then(|| &self.text[range])
    }

    fn replace_selection(&mut self, new_text: &str) -> bool {
        self.replace_text_in_range(None, new_text)
    }

    fn select_all(&mut self) {
        self.selection = Selection {
            anchor: 0,
            caret: self.text.len(),
        };
        self.marked = None;
    }

    fn move_left(&mut self, boundary: Boundary, extend: bool) {
        if !extend && !self.selection.is_empty() {
            let start = self.selection.range().start;
            self.selection = Selection {
                anchor: start,
                caret: start,
            };
            self.marked = None;
            return;
        }
        let target = match boundary {
            Boundary::Grapheme => previous_grapheme_boundary(&self.text, self.selection.caret),
            Boundary::Word => previous_word_boundary(&self.text, self.selection.caret),
            Boundary::Line => 0,
        };
        self.move_to(target, extend);
    }

    fn move_right(&mut self, boundary: Boundary, extend: bool) {
        if !extend && !self.selection.is_empty() {
            let end = self.selection.range().end;
            self.selection = Selection {
                anchor: end,
                caret: end,
            };
            self.marked = None;
            return;
        }
        let target = match boundary {
            Boundary::Grapheme => next_grapheme_boundary(&self.text, self.selection.caret),
            Boundary::Word => next_word_boundary(&self.text, self.selection.caret),
            Boundary::Line => self.text.len(),
        };
        self.move_to(target, extend);
    }

    fn move_to(&mut self, target: usize, extend: bool) {
        let target = target.min(self.text.len());
        if extend {
            self.selection.caret = target;
        } else {
            self.selection = Selection {
                anchor: target,
                caret: target,
            };
        }
        self.marked = None;
    }

    fn delete_left(&mut self, boundary: Boundary) -> bool {
        let selection = self.selection.range();
        let range = if selection.is_empty() {
            let start = match boundary {
                Boundary::Grapheme => previous_grapheme_boundary(&self.text, selection.start),
                Boundary::Word => previous_word_boundary(&self.text, selection.start),
                Boundary::Line => 0,
            };
            start..selection.start
        } else {
            selection
        };
        self.delete_range(range)
    }

    fn delete_right(&mut self, boundary: Boundary) -> bool {
        let selection = self.selection.range();
        let range = if selection.is_empty() {
            let end = match boundary {
                Boundary::Grapheme => next_grapheme_boundary(&self.text, selection.end),
                Boundary::Word => next_word_boundary(&self.text, selection.end),
                Boundary::Line => self.text.len(),
            };
            selection.end..end
        } else {
            selection
        };
        self.delete_range(range)
    }

    fn delete_range(&mut self, range: Range<usize>) -> bool {
        if range.is_empty() {
            return false;
        }
        self.text.replace_range(range.clone(), "");
        self.selection = Selection {
            anchor: range.start,
            caret: range.start,
        };
        self.marked = None;
        true
    }

    #[cfg(test)]
    fn selection(&self) -> Range<usize> {
        self.selection.range()
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Grapheme,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EditOutcome {
    pub handled: bool,
    pub content_changed: bool,
}

pub(crate) fn handle_key_down<T>(
    input: &mut TextInput,
    event: &KeyDownEvent,
    cx: &mut Context<T>,
) -> EditOutcome {
    let key = event.keystroke.key.as_str();
    let modifiers = event.keystroke.modifiers;
    if modifiers.platform {
        return match key {
            "a" => {
                input.select_all();
                handled(false)
            }
            "c" => {
                if let Some(text) = input.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                }
                handled(false)
            }
            "x" => {
                let changed = if let Some(text) = input.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    input.delete_left(Boundary::Grapheme)
                } else {
                    false
                };
                handled(changed)
            }
            "v" => {
                let changed = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .is_some_and(|text| input.replace_selection(&text));
                handled(changed)
            }
            "left" => {
                input.move_left(Boundary::Line, modifiers.shift);
                handled(false)
            }
            "right" => {
                input.move_right(Boundary::Line, modifiers.shift);
                handled(false)
            }
            "backspace" => handled(input.delete_left(Boundary::Line)),
            "delete" => handled(input.delete_right(Boundary::Line)),
            _ => EditOutcome::default(),
        };
    }
    if modifiers.alt {
        return match key {
            "left" => {
                input.move_left(Boundary::Word, modifiers.shift);
                handled(false)
            }
            "right" => {
                input.move_right(Boundary::Word, modifiers.shift);
                handled(false)
            }
            "backspace" => handled(input.delete_left(Boundary::Word)),
            "delete" => handled(input.delete_right(Boundary::Word)),
            _ => EditOutcome::default(),
        };
    }
    if modifiers.control {
        return EditOutcome::default();
    }
    if modifiers.function {
        return match key {
            "left" => {
                input.move_left(Boundary::Line, modifiers.shift);
                handled(false)
            }
            "right" => {
                input.move_right(Boundary::Line, modifiers.shift);
                handled(false)
            }
            "backspace" | "delete" => handled(input.delete_right(Boundary::Grapheme)),
            _ => EditOutcome::default(),
        };
    }
    match key {
        "left" => {
            input.move_left(Boundary::Grapheme, modifiers.shift);
            handled(false)
        }
        "right" => {
            input.move_right(Boundary::Grapheme, modifiers.shift);
            handled(false)
        }
        "home" => {
            input.move_left(Boundary::Line, modifiers.shift);
            handled(false)
        }
        "end" => {
            input.move_right(Boundary::Line, modifiers.shift);
            handled(false)
        }
        "backspace" => handled(input.delete_left(Boundary::Grapheme)),
        "delete" => handled(input.delete_right(Boundary::Grapheme)),
        _ => EditOutcome::default(),
    }
}

fn handled(content_changed: bool) -> EditOutcome {
    EditOutcome {
        handled: true,
        content_changed,
    }
}

pub(crate) fn render_text(input: &TextInput, show_caret: bool) -> AnyElement {
    let selection = if show_caret {
        input.selection.range()
    } else {
        0..0
    };
    let marked = show_caret
        .then(|| input.marked.as_ref().map(|marked| marked.range.clone()))
        .flatten();
    let caret = input.selection.caret;
    let mut content = div().flex().items_center().min_w_0().whitespace_nowrap();
    for (start, grapheme) in input.text.grapheme_indices(true) {
        if show_caret && caret == start {
            content = content.child(ui::input_caret());
        }
        let end = start + grapheme.len();
        content = content.child(
            div()
                .when(selection.start < end && start < selection.end, |text| {
                    text.bg(theme::accent_soft())
                })
                .when(
                    marked
                        .as_ref()
                        .is_some_and(|range| range.start < end && start < range.end),
                    |text| text.border_b_1().border_color(theme::accent()),
                )
                .child(grapheme.to_string()),
        );
    }
    if show_caret && caret == input.text.len() {
        content = content.child(ui::input_caret());
    }
    content.into_any_element()
}

fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

fn utf16_offset_to_utf8(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    for (utf8_offset, character) in text.char_indices() {
        if utf16_offset >= offset {
            return utf8_offset;
        }
        utf16_offset += character.len_utf16();
    }
    text.len()
}

fn utf16_range_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_offset_to_utf8(text, range.start)..utf16_offset_to_utf8(text, range.end)
}

fn utf8_range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    text[..range.start].encode_utf16().count()..text[..range.end].encode_utf16().count()
}

fn previous_grapheme_boundary(text: &str, position: usize) -> usize {
    text[..position.min(text.len())]
        .grapheme_indices(true)
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_grapheme_boundary(text: &str, position: usize) -> usize {
    let position = position.min(text.len());
    text[position..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(offset, _)| position + offset)
        .unwrap_or(text.len())
}

fn previous_word_boundary(text: &str, position: usize) -> usize {
    text.split_word_bound_indices()
        .take_while(|(start, _)| *start < position)
        .filter(|(_, segment)| is_word(segment))
        .map(|(start, _)| start)
        .fold(0, |_, start| start)
}

fn next_word_boundary(text: &str, position: usize) -> usize {
    text.split_word_bound_indices()
        .find(|(start, segment)| *start + segment.len() > position && is_word(segment))
        .map(|(start, segment)| start + segment.len())
        .unwrap_or(text.len())
}

fn is_word(segment: &str) -> bool {
    segment
        .chars()
        .any(|character| character.is_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_range_uses_utf16_offsets_and_moves_the_caret() {
        let mut input = TextInput::new("王😀菲");

        assert!(input.replace_text_in_range(Some(1..3), "逸成"));

        assert_eq!(input.text(), "王逸成菲");
        assert_eq!(input.selection(), "王逸成".len().."王逸成".len());
        assert_eq!(input.selected_text_range().range, 3..3);
    }

    #[test]
    fn marked_text_can_be_updated_committed_and_cancelled() {
        let mut input = TextInput::new("王菲");
        input.selection = Selection {
            anchor: "王".len(),
            caret: "王".len(),
        };

        assert!(input.replace_and_mark_text_in_range(None, "bianji", Some(6..6)));
        assert_eq!(input.text(), "王bianji菲");
        assert_eq!(input.marked_text_range(), Some(1..7));

        assert!(input.replace_and_mark_text_in_range(None, "编辑", Some(2..2)));
        assert_eq!(input.text(), "王编辑菲");
        assert_eq!(input.marked_text_range(), Some(1..3));

        assert!(!input.replace_text_in_range(None, "编辑"));
        assert_eq!(input.marked_text_range(), None);
        assert_eq!(input.text(), "王编辑菲");

        assert!(input.replace_and_mark_text_in_range(Some(1..3), "yicheng", None));
        assert!(input.cancel_marked_text());
        assert_eq!(input.text(), "王编辑菲");
        assert_eq!(input.marked_text_range(), None);
    }

    #[test]
    fn marked_text_selection_keeps_the_caret_at_the_range_end() {
        let mut input = TextInput::new("");

        assert!(input.replace_and_mark_text_in_range(None, "かな", Some(0..1)));

        assert_eq!(input.selected_text_range().range, 0..1);
        assert!(!input.selected_text_range().reversed);
        assert_eq!(input.character_index_utf16(), 1);
    }

    #[test]
    fn caret_and_selection_follow_character_word_and_line_boundaries() {
        let mut input = TextInput::new("alpha 王菲 beta");

        input.move_left(Boundary::Word, false);
        assert_eq!(input.selection(), 13..13);
        input.move_left(Boundary::Grapheme, false);
        assert_eq!(input.selection(), 12..12);
        input.move_left(Boundary::Line, true);
        assert_eq!(input.selection(), 0..12);
        assert!(input.selected_text_range().reversed);
        input.move_right(Boundary::Line, false);
        assert_eq!(input.selection(), 12..12);
        input.select_all();
        assert_eq!(input.selection(), 0..input.text().len());
        input.move_left(Boundary::Grapheme, false);
        assert_eq!(input.selection(), 0..0);
    }

    #[test]
    fn mid_string_delete_respects_graphemes_and_selections() {
        let mut input = TextInput::new("A👨‍👩‍👧‍👦B");
        input.move_left(Boundary::Grapheme, false);
        assert!(input.delete_left(Boundary::Grapheme));
        assert_eq!(input.text(), "AB");

        input.selection = Selection {
            anchor: 0,
            caret: 1,
        };
        assert!(input.delete_right(Boundary::Grapheme));
        assert_eq!(input.text(), "B");
        assert_eq!(input.selection(), 0..0);

        input.reset("ABC");
        input.selection = Selection {
            anchor: 1,
            caret: 1,
        };
        assert!(input.delete_right(Boundary::Grapheme));
        assert_eq!(input.text(), "AC");
        assert_eq!(input.selection(), 1..1);
    }
}
