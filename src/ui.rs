use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, mpsc::Sender},
};

use gpui::prelude::*;
use gpui::{
    Context, Decorations, Entity, FocusHandle, IntoElement, MouseButton, PathBuilder, Render,
    SharedString, Window, WindowControlArea, canvas, div, img, point, px, rgb,
};

use crate::{
    Event, ModelKind,
    config::{Theme, Trigger, display_hotkey},
    field::Field,
};

pub const ACCENT: u32 = 0x00cbff;

fn pick_image(
    dark: bool,
    image: &Arc<gpui::RenderImage>,
    light_image: &Arc<gpui::RenderImage>,
) -> Arc<gpui::RenderImage> {
    if dark {
        image.clone()
    } else {
        light_image.clone()
    }
}

pub struct Root {
    pub tx: Sender<Event>,
    pub shortcuts: [Trigger; 3],
    pub shortcut_focus: FocusHandle,
    pub recording: Option<usize>,
    pub shortcut_modifiers: gpui::Modifiers,
    pub status: SharedString,
    pub theme: Rc<Cell<Theme>>,
    key: Entity<Field>,
    voice: Entity<Field>,
    answer: Entity<Field>,
    mic: Entity<Field>,
    logo: Arc<gpui::RenderImage>,
    light_logo: Arc<gpui::RenderImage>,
}

impl Root {
    fn toggle_maximize(window: &Window) {
        #[cfg(windows)]
        if window.is_maximized() {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            use windows_sys::Win32::UI::WindowsAndMessaging::{SW_RESTORE, ShowWindow};
            if let Ok(raw) = HasWindowHandle::window_handle(window)
                && let RawWindowHandle::Win32(raw) = raw.as_raw()
            {
                unsafe {
                    ShowWindow(raw.hwnd.get() as *mut _, SW_RESTORE);
                }
            }
        } else {
            window.zoom_window();
        }
        #[cfg(not(windows))]
        window.zoom_window();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tx: Sender<Event>,
        shortcuts: [Trigger; 3],
        shortcut_focus: FocusHandle,
        theme: Rc<Cell<Theme>>,
        key: Entity<Field>,
        voice: Entity<Field>,
        answer: Entity<Field>,
        mic: Entity<Field>,
        logo: Arc<gpui::RenderImage>,
        light_logo: Arc<gpui::RenderImage>,
    ) -> Self {
        Self {
            tx,
            shortcuts,
            shortcut_focus,
            recording: None,
            shortcut_modifiers: gpui::Modifiers::default(),
            status: "Ready".into(),
            theme,
            key,
            voice,
            answer,
            mic,
            logo,
            light_logo,
        }
    }

    fn button(
        tx: Sender<Event>,
        event: Event,
        label: &str,
        dark: bool,
        selected: bool,
    ) -> impl IntoElement {
        let id = match &event {
            Event::RefreshModels(ModelKind::Voice) => "voice-browse".to_owned(),
            Event::RefreshModels(ModelKind::Answer) => "answer-browse".to_owned(),
            Event::RefreshMics => "mic-browse".to_owned(),
            Event::CaptureStart(index) => format!("shortcut-{index}"),
            _ => label.to_owned(),
        };
        let model_picker = matches!(
            &event,
            Event::RefreshModels(_) | Event::RefreshMics | Event::ClearKey
        );
        div()
            .id(SharedString::from(id))
            .flex()
            .items_center()
            .justify_center()
            .px_3()
            .h(px(32.))
            .when(model_picker, |button| button.w(px(80.)).flex_shrink_0())
            .rounded_md()
            .border_1()
            .border_color(rgb(if selected {
                ACCENT
            } else if dark {
                0x414b59
            } else {
                0xcbd4df
            }))
            .bg(rgb(if selected {
                ACCENT
            } else if dark {
                0x1a1e24
            } else {
                0xffffff
            }))
            .hover(|style| {
                style.bg(rgb(if selected {
                    0x00b9e6
                } else if dark {
                    0x232b36
                } else {
                    0xe8eef4
                }))
            })
            .active(|style| style.opacity(0.85))
            .text_color(rgb(if selected {
                0x000000
            } else if dark {
                0xf3f5f7
            } else {
                0x161b24
            }))
            .cursor_pointer()
            .child(label.to_string())
            .on_click(move |_, _, _| {
                let _ = tx.send(event.clone());
            })
    }

    fn setting_row(
        label: &str,
        field: Entity<Field>,
        picker: Option<(ModelKind, &str)>,
        tx: &Sender<Event>,
        dark: bool,
        muted: u32,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(116.))
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(rgb(muted))
                    .child(label.to_owned()),
            )
            .child(field)
            .when(label == "API key", |row| {
                row.child(Self::button(
                    tx.clone(),
                    Event::ClearKey,
                    "Clear",
                    dark,
                    false,
                ))
            })
            .when_some(picker, |row, (kind, label)| {
                row.child(Self::button(
                    tx.clone(),
                    Event::RefreshModels(kind),
                    label,
                    dark,
                    false,
                ))
            })
    }

    fn window_control(
        id: &'static str,
        area: WindowControlArea,
        maximized: bool,
        dark: bool,
        settings_tx: Option<Sender<Event>>,
    ) -> impl IntoElement {
        let color = if dark { 0xe8edf4 } else { 0x252d36 };
        let glyph = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let x = bounds.origin.x;
                let y = bounds.origin.y;
                let mut path = PathBuilder::stroke(px(1.2));
                match area {
                    WindowControlArea::Min => {}
                    WindowControlArea::Max if maximized => {
                        path.move_to(point(x + px(5.), y + px(3.)));
                        path.line_to(point(x + px(12.), y + px(3.)));
                        path.line_to(point(x + px(12.), y + px(10.)));
                        path.move_to(point(x + px(2.), y + px(5.)));
                        path.line_to(point(x + px(9.), y + px(5.)));
                        path.line_to(point(x + px(9.), y + px(12.)));
                        path.line_to(point(x + px(2.), y + px(12.)));
                        path.close();
                    }
                    WindowControlArea::Max => {
                        path.move_to(point(x + px(2.), y + px(2.)));
                        path.line_to(point(x + px(12.), y + px(2.)));
                        path.line_to(point(x + px(12.), y + px(12.)));
                        path.line_to(point(x + px(2.), y + px(12.)));
                        path.close();
                    }
                    WindowControlArea::Close => {
                        path.move_to(point(x + px(3.), y + px(3.)));
                        path.line_to(point(x + px(11.), y + px(11.)));
                        path.move_to(point(x + px(11.), y + px(3.)));
                        path.line_to(point(x + px(3.), y + px(11.)));
                    }
                    WindowControlArea::Drag => {}
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            },
        )
        .size(px(14.));
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(44.))
            .h_full()
            .hover(move |style| {
                style.bg(rgb(if matches!(area, WindowControlArea::Close) {
                    0xc42b1c
                } else if dark {
                    0x343b45
                } else {
                    0xe1e7ee
                }))
            })
            .active(|style| style.opacity(0.8))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match area {
                    WindowControlArea::Min => window.minimize_window(),
                    WindowControlArea::Max => Self::toggle_maximize(window),
                    WindowControlArea::Close => {
                        if let Some(tx) = &settings_tx {
                            let _ = tx.send(Event::CancelSettings);
                        } else {
                            window.remove_window();
                        }
                    }
                    WindowControlArea::Drag => {}
                }
            })
            .when(matches!(area, WindowControlArea::Min), |button| {
                button.child(div().w(px(10.)).h(px(1.)).bg(rgb(color)))
            })
            .when(!matches!(area, WindowControlArea::Min), |button| {
                button.child(glyph)
            })
    }

    fn title_bar(
        logo: Arc<gpui::RenderImage>,
        window: &mut Window,
        dark: bool,
        settings_tx: Option<Sender<Event>>,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .h(px(40.))
            .flex_shrink_0()
            .bg(rgb(if dark { 0x111419 } else { 0xf7f9fb }))
            .child(
                div()
                    .id("title-drag")
                    .flex()
                    .items_center()
                    .flex_1()
                    .h_full()
                    .pl(if cfg!(target_os = "macos") {
                        px(80.)
                    } else {
                        px(12.)
                    })
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            if cfg!(target_os = "macos") {
                                window.titlebar_double_click();
                            } else {
                                Self::toggle_maximize(window);
                            }
                        } else {
                            #[cfg(windows)]
                            {
                                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                                use windows_sys::Win32::UI::{
                                    Input::KeyboardAndMouse::ReleaseCapture,
                                    WindowsAndMessaging::{
                                        HTCAPTION, PostMessageW, SC_MOVE, WM_SYSCOMMAND,
                                    },
                                };
                                if let Ok(raw) = HasWindowHandle::window_handle(window)
                                    && let RawWindowHandle::Win32(raw) = raw.as_raw()
                                {
                                    unsafe {
                                        ReleaseCapture();
                                        PostMessageW(
                                            raw.hwnd.get() as *mut _,
                                            WM_SYSCOMMAND,
                                            (SC_MOVE | HTCAPTION) as usize,
                                            0,
                                        );
                                    }
                                }
                            }
                            #[cfg(not(windows))]
                            window.start_window_move();
                        }
                    })
                    .child(img(logo).w(px(76.)).h(px(28.5))),
            )
            .when(
                !cfg!(target_os = "macos")
                    && (cfg!(target_os = "windows")
                        || matches!(window.window_decorations(), Decorations::Client { .. })),
                |bar| {
                    let controls = window.window_controls();
                    bar.when(controls.minimize, |bar| {
                        bar.child(Self::window_control(
                            "minimize",
                            WindowControlArea::Min,
                            false,
                            dark,
                            None,
                        ))
                    })
                    .when(controls.maximize, |bar| {
                        bar.child(Self::window_control(
                            "maximize",
                            WindowControlArea::Max,
                            window.is_maximized(),
                            dark,
                            None,
                        ))
                    })
                    .child(Self::window_control(
                        "close",
                        WindowControlArea::Close,
                        false,
                        dark,
                        settings_tx,
                    ))
                },
            )
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.theme.get().dark(window.appearance());
        let bg = if dark { 0x111419 } else { 0xf7f9fb };
        let fg = if dark { 0xf3f5f7 } else { 0x161b24 };
        let muted = if dark { 0xaab5c3 } else { 0x526071 };
        let border = if dark { 0x343d4a } else { 0xd8e0e8 };
        let tx = self.tx.clone();
        let title = Self::title_bar(
            pick_image(dark, &self.logo, &self.light_logo),
            window,
            dark,
            Some(self.tx.clone()),
        );
        let contributor = self.answer.read(cx).value().contains("contributor");
        let scroll =
            div()
                .id("settings")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .bg(rgb(bg))
                .text_color(rgb(fg))
                .child(Self::setting_row(
                    "API key",
                    self.key.clone(),
                    None,
                    &tx,
                    dark,
                    muted,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(116.))
                                .flex_shrink_0()
                                .text_sm()
                                .text_color(rgb(muted))
                                .child("Microphone".to_owned()),
                        )
                        .child(self.mic.clone())
                        .child(Self::button(
                            tx.clone(),
                            Event::RefreshMics,
                            "Browse",
                            dark,
                            false,
                        )),
                )
                .child(Self::setting_row(
                    "Dictation model",
                    self.voice.clone(),
                    Some((ModelKind::Voice, "Browse")),
                    &tx,
                    dark,
                    muted,
                ))
                .child(Self::setting_row(
                    "Answer model",
                    self.answer.clone(),
                    Some((ModelKind::Answer, "Browse")),
                    &tx,
                    dark,
                    muted,
                ))
                .when(contributor, |row| {
                    row.child(div().text_sm().text_color(rgb(muted)).child(
                        "Contributor models let Meta store submitted content and responses.",
                    ))
                })
                .child(div().h(px(1.)).w_full().bg(rgb(border)))
                .child(div().text_sm().text_color(rgb(muted)).child("Shortcuts"))
                .children(self.shortcuts.iter().enumerate().map(|(index, shortcut)| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(shortcut.name.clone())
                        .child(Self::button(
                            tx.clone(),
                            Event::CaptureStart(index),
                            &if self.recording == Some(index) {
                                let m = self.shortcut_modifiers;
                                let mods: Vec<_> = [
                                    (m.control, "Ctrl"),
                                    (m.alt, "Alt"),
                                    (m.shift, "Shift"),
                                    (
                                        m.platform,
                                        if cfg!(target_os = "macos") {
                                            "Cmd"
                                        } else {
                                            "Super"
                                        },
                                    ),
                                ]
                                .into_iter()
                                .filter_map(|(on, name)| on.then_some(name))
                                .collect();
                                if mods.is_empty() {
                                    "Press a shortcut".into()
                                } else {
                                    format!("{} + …", mods.join(" + "))
                                }
                            } else {
                                display_hotkey(shortcut)
                            },
                            dark,
                            self.recording == Some(index),
                        ))
                }))
                .child(div().h(px(1.)).w_full().bg(rgb(border)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().mr_2().text_sm().text_color(rgb(muted)).child("Theme"))
                        .child(Self::button(
                            tx.clone(),
                            Event::SetTheme(Theme::System),
                            "System",
                            dark,
                            self.theme.get() == Theme::System,
                        ))
                        .child(Self::button(
                            tx.clone(),
                            Event::SetTheme(Theme::Light),
                            "Light",
                            dark,
                            self.theme.get() == Theme::Light,
                        ))
                        .child(Self::button(
                            tx.clone(),
                            Event::SetTheme(Theme::Dark),
                            "Dark",
                            dark,
                            self.theme.get() == Theme::Dark,
                        )),
                );
        let footer = div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(border))
            .px_4()
            .py_2()
            .bg(rgb(bg))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(muted))
                    .child(self.status.clone()),
            )
            .child(Self::button(
                tx.clone(),
                Event::CancelSettings,
                "Cancel",
                dark,
                false,
            ))
            .child(Self::button(tx, Event::SaveProvider, "Save", dark, true));
        div()
            .track_focus(&self.shortcut_focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if this.recording.is_some() && !event.is_held {
                    let _ = this.tx.send(Event::CaptureKey(event.keystroke.clone()));
                    cx.stop_propagation();
                }
            }))
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                    if this.recording.is_some() {
                        this.shortcut_modifiers = event.modifiers;
                        cx.notify();
                    }
                },
            ))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(bg))
            .child(title)
            .child(scroll)
            .child(footer)
    }
}

pub struct ModelBrowser {
    pub models: Vec<String>,
    pub others: Vec<String>,
    pub others_open: bool,
    pub kind: ModelKind,
    pub tx: Sender<Event>,
    pub theme: Rc<Cell<Theme>>,
    pub logo: Arc<gpui::RenderImage>,
    pub light_logo: Arc<gpui::RenderImage>,
}

impl ModelBrowser {
    fn model_row(model: &str, kind: ModelKind, tx: &Sender<Event>, dark: bool) -> impl IntoElement {
        let model = model.to_owned();
        let tx = tx.clone();
        div()
            .id(SharedString::from(model.clone()))
            .px_2()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(if dark { 0x29323d } else { 0xe4f2f7 })))
            .active(|style| style.opacity(0.85))
            .child(model.clone())
            .on_click(move |_, _, _| {
                let _ = tx.send(Event::SelectModel(kind, model.clone()));
            })
    }
}

impl Render for ModelBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.theme.get().dark(window.appearance());
        let bg = if dark { 0x111419 } else { 0xf7f9fb };
        let fg = if dark { 0xf3f5f7 } else { 0x161b24 };
        let muted = if dark { 0xaab5c3 } else { 0x526071 };
        let title = Root::title_bar(
            pick_image(dark, &self.logo, &self.light_logo),
            window,
            dark,
            None,
        );
        let kind = self.kind;
        let tx = self.tx.clone();
        let open = self.others_open;
        let chevron = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let x = bounds.origin.x;
                let y = bounds.origin.y;
                let mut path = PathBuilder::stroke(px(1.4));
                if open {
                    path.move_to(point(x + px(2.), y + px(4.)));
                    path.line_to(point(x + px(6.), y + px(8.)));
                    path.line_to(point(x + px(10.), y + px(4.)));
                } else {
                    path.move_to(point(x + px(4.), y + px(2.)));
                    path.line_to(point(x + px(8.), y + px(6.)));
                    path.line_to(point(x + px(4.), y + px(10.)));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(fg));
                }
            },
        )
        .size(px(12.));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .child(title)
            .child(
                div()
                    .id("models")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .px_2()
                    .py_2()
                    .when(self.models.is_empty(), |list| {
                        list.child(div().px_2().py_2().child("No matching models available"))
                    })
                    .children(
                        self.models
                            .iter()
                            .map(|model| Self::model_row(model, kind, &tx, dark)),
                    )
                    .when(!self.others.is_empty(), |list| {
                        list.child(
                            div()
                                .id("other-models")
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_2()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|style| {
                                    style.bg(rgb(if dark { 0x29323d } else { 0xe4f2f7 }))
                                })
                                .active(|style| style.opacity(0.85))
                                .child(chevron)
                                .child("Others")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.others_open = !this.others_open;
                                    cx.notify();
                                })),
                        )
                        .when(open, |list| {
                            list.child(
                                div()
                                    .px_2()
                                    .text_sm()
                                    .text_color(rgb(muted))
                                    .child("May not work for this task"),
                            )
                            .children(
                                self.others
                                    .iter()
                                    .map(|model| Self::model_row(model, kind, &tx, dark)),
                            )
                        })
                    }),
            )
    }
}

pub struct MicBrowser {
    pub mics: Vec<String>,
    pub tx: Sender<Event>,
    pub theme: Rc<Cell<Theme>>,
    pub logo: Arc<gpui::RenderImage>,
    pub light_logo: Arc<gpui::RenderImage>,
}

impl Render for MicBrowser {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.theme.get().dark(window.appearance());
        let bg = if dark { 0x111419 } else { 0xf7f9fb };
        let fg = if dark { 0xf3f5f7 } else { 0x161b24 };
        let title = Root::title_bar(
            pick_image(dark, &self.logo, &self.light_logo),
            window,
            dark,
            None,
        );
        let tx = self.tx.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .child(title)
            .child(
                div()
                    .id("mics")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .px_2()
                    .py_2()
                    .when(self.mics.is_empty(), |list| {
                        list.child(div().px_2().py_2().child("No microphones available"))
                    })
                    .children(self.mics.iter().map(|mic| {
                        let mic = mic.clone();
                        let tx = tx.clone();
                        div()
                            .id(SharedString::from(mic.clone()))
                            .px_2()
                            .py_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| {
                                style.bg(rgb(if dark { 0x29323d } else { 0xe4f2f7 }))
                            })
                            .active(|style| style.opacity(0.85))
                            .child(mic.clone())
                            .on_click(move |_, _, _| {
                                let _ = tx.send(Event::SelectMic(mic.clone()));
                            })
                    })),
            )
    }
}

pub struct ErrorDialog {
    pub message: SharedString,
    pub tx: Sender<Event>,
    pub theme: Rc<Cell<Theme>>,
    pub logo: Arc<gpui::RenderImage>,
    pub light_logo: Arc<gpui::RenderImage>,
}

impl Render for ErrorDialog {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.theme.get().dark(window.appearance());
        let bg = if dark { 0x111419 } else { 0xf7f9fb };
        let fg = if dark { 0xf3f5f7 } else { 0x161b24 };
        let border = if dark { 0x343d4a } else { 0xd8e0e8 };
        let title = Root::title_bar(
            pick_image(dark, &self.logo, &self.light_logo),
            window,
            dark,
            None,
        );
        let tx = self.tx.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .child(title)
            .child(
                div()
                    .id("error-message")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .px_4()
                    .py_3()
                    .child(div().text_sm().child(self.message.clone())),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(border))
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(Root::button(tx, Event::DismissError, "Close", dark, true)),
            )
    }
}

pub struct AnswerCard {
    pub text: SharedString,
    pub theme: Rc<Cell<Theme>>,
    pub tx: Sender<Event>,
    pub seq: u64,
}

impl Render for AnswerCard {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.theme.get().dark(window.appearance());
        let tx = self.tx.clone();
        let seq = self.seq;
        // see Indicator
        div()
            .id("answer-card")
            .flex()
            .flex_col()
            .w(px(340.))
            .h(px(140.))
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if dark { 0x414b59 } else { 0xcbd4df }))
            .bg(rgb(if dark { 0x171c23 } else { 0xf7f9fb }))
            .text_color(rgb(if dark { 0xf3f5f7 } else { 0x161b24 }))
            .cursor_pointer()
            .child(
                div()
                    .id("answer-text")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .text_sm()
                    .child(self.text.clone()),
            )
            .on_click(move |_, _, _| {
                let _ = tx.send(Event::DismissAnswer(seq));
            })
    }
}

pub struct Indicator {
    pub status: SharedString,
    pub theme: Rc<Cell<Theme>>,
    pub icon: Arc<gpui::RenderImage>,
    pub icon_light: Arc<gpui::RenderImage>,
}

impl Render for Indicator {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        {
            let b = window.bounds();
            let v = window.viewport_size();
            crate::log_event(&format!(
                "indicator render bounds={}x{}@{},{} viewport={}x{} scale={} status={:?}",
                f32::from(b.size.width),
                f32::from(b.size.height),
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(v.width),
                f32::from(v.height),
                window.scale_factor(),
                self.status,
            ));
        }
        let dark = self.theme.get().dark(window.appearance());
        div()
            .flex()
            .items_center()
            .gap_2()
            .w(px(260.))
            .h(px(36.))
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(if dark { 0x414b59 } else { 0xcbd4df }))
            .bg(rgb(if dark { 0x171c23 } else { 0xf7f9fb }))
            .child(
                div()
                    .flex_shrink_0()
                    .child(img(pick_image(dark, &self.icon, &self.icon_light)).size(px(20.))),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(if dark { 0xf3f5f7 } else { 0x161b24 }))
                    .pt(px(1.))
                    .child(self.status.clone()),
            )
    }
}
