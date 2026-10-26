use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ContentMask, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, Pixels, Point, Render, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, Window, actions, div, fill, hsla, point, px, relative, rgb, size,
};

use crate::{Event, config::Theme};

actions!(field, [Backspace, Delete, Left, Right, Home, End, Paste]);

pub struct Field {
    value: String,
    placeholder: SharedString,
    masked: bool,
    strip_spaces: bool,
    cursor: usize,
    scroll_x: Cell<Pixels>,
    focus: FocusHandle,
    tx: Sender<Event>,
    theme: Rc<Cell<Theme>>,
}

impl Field {
    pub fn new(
        value: &str,
        placeholder: &str,
        masked: bool,
        strip_spaces: bool,
        tx: Sender<Event>,
        theme: Rc<Cell<Theme>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            cursor: value.len(),
            scroll_x: Cell::new(px(0.)),
            value: value.into(),
            placeholder: placeholder.to_owned().into(),
            masked,
            strip_spaces,
            focus: cx.focus_handle(),
            tx,
            theme,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set(&mut self, value: String, cx: &mut Context<Self>) {
        self.cursor = value.len();
        self.value = value;
        cx.notify();
    }

    fn clamp(&mut self) {
        self.cursor = self.value.floor_char_boundary(self.cursor);
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        // microphone names can have spaces
        let clean: String = if self.strip_spaces {
            text.split_whitespace().collect()
        } else {
            text.to_string()
        };
        self.clamp();
        self.value.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        cx.notify();
        let _ = self.tx.send(Event::FieldsChanged);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.clamp();
        let prev = self.value[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        if prev < self.cursor {
            self.value.drain(prev..self.cursor);
            self.cursor = prev;
            cx.notify();
            let _ = self.tx.send(Event::FieldsChanged);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.clamp();
        let next = self.value[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.value.len());
        if next > self.cursor {
            self.value.drain(self.cursor..next);
            cx.notify();
            let _ = self.tx.send(Event::FieldsChanged);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.clamp();
        self.cursor = self.value[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        cx.notify();
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.clamp();
        self.cursor = self.value[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.value.len());
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = 0;
        cx.notify();
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = self.value.len();
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text, cx);
        }
    }

    fn focus_me(&mut self, _: &MouseDownEvent, window: &mut Window, _: &mut Context<Self>) {
        window.focus(&self.focus);
    }
}

impl Focusable for Field {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Field {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let dark = self.theme.get().dark(window.appearance());
        div()
            .key_context("field")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_me))
            .flex_1()
            .min_w(px(0.))
            .h(px(32.))
            .flex()
            .items_center()
            .px_2()
            .bg(rgb(if dark { 0x1a1e24 } else { 0xffffff }))
            .text_color(rgb(if dark { 0xf3f5f7 } else { 0x161b24 }))
            .rounded_md()
            .border_1()
            .border_color(rgb(if focused {
                crate::ui::ACCENT
            } else if dark {
                0x414b59
            } else {
                0xcdd5df
            }))
            .child(FieldText { field: cx.entity() })
    }
}

struct FieldText {
    field: Entity<Field>,
}

struct TextPaint {
    line: Option<ShapedLine>,
    cursor: Option<gpui::PaintQuad>,
    scroll_x: Pixels,
}

impl IntoElement for FieldText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FieldText {
    type RequestLayoutState = ();
    type PrepaintState = TextPaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let field = self.field.read(cx);
        let display = if field.value.is_empty() {
            field.placeholder.clone()
        } else if field.masked {
            "•".repeat(field.value.chars().count()).into()
        } else {
            field.value.clone().into()
        };
        let color = if field.value.is_empty() {
            hsla(0., 0., 0.55, 1.)
        } else {
            rgb(if field.theme.get().dark(window.appearance()) {
                0xf3f5f7
            } else {
                0x161b24
            })
            .into()
        };
        let style = window.text_style();
        let run = TextRun {
            len: display.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display, font_size, &[run], None);
        let cursor_x = line.x_for_index(
            if field.masked {
                field.value[..field.cursor].chars().count() * "•".len()
            } else {
                field.cursor
            }
            .min(line.len()),
        );
        let scroll_x = scroll_to_cursor(
            field.scroll_x.get(),
            cursor_x,
            bounds.size.width,
            line.width,
        );
        field.scroll_x.set(scroll_x);
        let cursor = fill(
            Bounds::new(
                point(bounds.left() + cursor_x - scroll_x, bounds.top()),
                size(px(2.), bounds.bottom() - bounds.top()),
            ),
            rgb(crate::ui::ACCENT),
        );
        TextPaint {
            line: Some(line),
            cursor: Some(cursor),
            scroll_x,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let origin = point(bounds.origin.x - prepaint.scroll_x, bounds.origin.y);
            prepaint
                .line
                .take()
                .unwrap()
                .paint(origin, window.line_height(), window, cx)
                .unwrap();
            if focus.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
    }
}

impl EntityInputHandler for Field {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf16_to_utf8(&self.value, range_utf16.start);
        let end = utf16_to_utf8(&self.value, range_utf16.end);
        actual_range.replace(range_utf16);
        Some(self.value[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert(text, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

fn utf16_to_utf8(s: &str, utf16: usize) -> usize {
    let mut count = 0;
    for (i, ch) in s.char_indices() {
        if count >= utf16 {
            return i;
        }
        count += ch.len_utf16();
    }
    s.len()
}

fn scroll_to_cursor(offset: Pixels, cursor: Pixels, width: Pixels, text_width: Pixels) -> Pixels {
    let width = (width - px(2.)).max(px(0.));
    let offset = offset.min((text_width - width).max(px(0.)));
    if cursor < offset {
        cursor
    } else if cursor > offset + width {
        cursor - width
    } else {
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_long_input_caret_visible() {
        assert_eq!(
            scroll_to_cursor(px(0.), px(180.), px(100.), px(200.)),
            px(82.)
        );
        assert_eq!(
            scroll_to_cursor(px(82.), px(50.), px(100.), px(200.)),
            px(50.)
        );
        assert_eq!(
            scroll_to_cursor(px(82.), px(90.), px(100.), px(200.)),
            px(82.)
        );
        assert_eq!(
            scroll_to_cursor(px(82.), px(20.), px(100.), px(20.)),
            px(0.)
        );
    }
}
