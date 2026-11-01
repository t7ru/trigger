#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod capture;
mod config;
mod field;
mod muse;
mod ui;

use config::{Config, MUSE_BASE_URL, MUSE_MODEL, Theme, VOICE_MODEL};
use field::Field;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, SharedString, Timer, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions, point,
    px, size,
};
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        mpsc::{self, TryRecvError},
    },
    time::Duration,
};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};
use ui::{AnswerCard, ErrorDialog, Indicator, MicBrowser, ModelBrowser, Root};

#[cfg(debug_assertions)]
pub(crate) fn log_event(message: &str) {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    eprintln!("[trigger {} {ms}] {message}", std::process::id());
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn log_event(_message: &str) {}

fn render_image(bytes: &[u8]) -> Arc<gpui::RenderImage> {
    let mut image = image::load_from_memory(bytes)
        .expect("embedded image")
        .thumbnail(512, 256)
        .to_rgba8();
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(gpui::RenderImage::new([image::Frame::new(image)]))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Voice,
    Answer,
}

impl ModelKind {
    fn supports(self, model: &str) -> bool {
        match self {
            Self::Voice => model.starts_with("muse-voice-transcribe-"),
            Self::Answer => model.starts_with("muse-spark-") || model.starts_with("muse-wonder-"),
        }
    }
}

#[cfg(test)]
mod model_tests {
    use super::ModelKind;

    #[test]
    fn classifies_meta_model_families() {
        assert!(ModelKind::Voice.supports("muse-voice-transcribe-1.0"));
        assert!(ModelKind::Voice.supports("muse-voice-transcribe-2.0"));
        for model in [
            "muse-wonder-1.0",
            "muse-spark-1.3",
            "muse-spark-1.3-contributor",
            "muse-spark-1.2",
            "muse-spark-1.2-contributor",
            "muse-spark-1.1",
            "muse-spark-1.4",
            "muse-wonder-2.0",
        ] {
            assert!(ModelKind::Answer.supports(model));
            assert!(!ModelKind::Voice.supports(model));
        }
        for model in ["muse-image-1.0", "sam-3.1", "muse-sparky-1.0"] {
            assert!(!ModelKind::Voice.supports(model));
            assert!(!ModelKind::Answer.supports(model));
        }
        assert!(!ModelKind::Answer.supports("muse-voice-transcribe-1.0"));
    }
}

#[derive(Clone)]
pub enum Event {
    Trigger(u32),
    Show,
    Quit,
    FieldsChanged,
    ClearKey,
    CaptureStart(usize),
    CaptureKey(gpui::Keystroke),
    CaptureEnd,
    Stage(&'static str),
    SaveProvider,
    CancelSettings,
    RefreshModels(ModelKind),
    Models(u64, ModelKind, String, Result<Vec<String>, String>),
    SelectModel(ModelKind, String),
    RefreshMics,
    SelectMic(String),
    DismissAnswer(u64),
    DismissError,
    SetTheme(Theme),
    SetAutoPaste(bool),
    SetScreenshot(bool),
    Done {
        label: String,
        target: isize,
        card: bool,
        result: Result<String, String>,
    },
    Pasted(Result<(), String>),
}

type FieldEntity = gpui::Entity<Field>;

struct Settings {
    tx: mpsc::Sender<Event>,
    root: gpui::Entity<Root>,
    window: Option<gpui::WindowHandle<Root>>,
    key: FieldEntity,
    voice: FieldEntity,
    answer: FieldEntity,
    mic: FieldEntity,
    theme: Rc<Cell<Theme>>,
    applied_theme: Rc<Cell<Theme>>,
    auto_paste: Rc<Cell<bool>>,
    screenshot: Rc<Cell<bool>>,
    model_request: u64,
    model_cache: HashMap<String, Vec<String>>,
    indicator: Option<gpui::WindowHandle<Indicator>>,
    browser: Option<gpui::WindowHandle<ModelBrowser>>,
    browser_key: Option<String>,
    mic_browser: Option<gpui::WindowHandle<MicBrowser>>,
    error: Option<gpui::WindowHandle<ErrorDialog>>,
    card: Option<gpui::WindowHandle<AnswerCard>>,
    card_seq: u64,
    logo: Arc<gpui::RenderImage>,
    light_logo: Arc<gpui::RenderImage>,
    icon: Arc<gpui::RenderImage>,
    icon_light: Arc<gpui::RenderImage>,
    manager: GlobalHotKeyManager,
    hotkeys: [Option<HotKey>; 3],
    capturing: bool,
}

fn window_options(
    bounds: Bounds<gpui::Pixels>,
    min_size: gpui::Size<gpui::Pixels>,
    title: &'static str,
) -> WindowOptions {
    WindowOptions {
        app_id: Some("trigger".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(min_size),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: true,
            ..Default::default()
        }),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

fn popup_options(bounds: Bounds<gpui::Pixels>) -> WindowOptions {
    WindowOptions {
        app_id: Some("trigger".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        kind: WindowKind::PopUp,
        titlebar: None,
        focus: false,
        show: !cfg!(windows),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

fn observed<V: gpui::Render + 'static>(
    window: &mut gpui::Window,
    cx: &mut App,
    view: V,
) -> gpui::Entity<V> {
    cx.new(|cx| {
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        view
    })
}

impl Settings {
    fn show(&mut self, cx: &mut App) {
        if self.window.as_ref().is_some_and(|w| {
            w.update(cx, |_, window, _| {
                #[cfg(windows)]
                show_settings(window, true);
                window.activate_window();
            })
            .is_ok()
        }) {
            return;
        }
        let root = self.root.clone();
        let tx = self.tx.clone();
        let fields = [
            self.key.clone(),
            self.voice.clone(),
            self.answer.clone(),
            self.mic.clone(),
        ];
        let bounds = Bounds::centered(None, size(px(540.), px(480.)), cx);
        self.window = Some(
            cx.open_window(
                window_options(bounds, size(px(480.), px(340.)), "trigger"),
                move |window, _cx| {
                    window.on_window_should_close(_cx, move |_, _| {
                        let _ = tx.send(Event::CancelSettings);
                        false
                    });
                    let refresh = root.clone();
                    root.update(_cx, |view, cx| {
                        let focus = view.shortcut_focus.clone();
                        cx.on_blur(&focus, window, |view, _, cx| {
                            if view.recording.take().is_some() {
                                let _ = view.tx.send(Event::CaptureEnd);
                                cx.notify();
                            }
                        })
                        .detach();
                    });
                    window
                        .observe_window_appearance(move |_, cx| {
                            refresh.update(cx, |_, cx| cx.notify());
                            for field in &fields {
                                field.update(cx, |_, cx| cx.notify());
                            }
                        })
                        .detach();
                    root
                },
            )
            .expect("open settings"),
        );
    }

    fn status(&self, cx: &mut App, status: String) {
        self.root.update(cx, |root, cx| {
            root.status = status.into();
            cx.notify();
        });
    }

    fn refresh_status(&self, cx: &mut App, config: &Config) {
        let changed = self.key.read(cx).value().trim() != config.api_key
            || self.voice.read(cx).value().trim() != config.voice_model
            || self.answer.read(cx).value().trim() != config.model
            || self.mic.read(cx).value().trim() != config.mic
            || self.theme.get() != config.theme
            || self.auto_paste.get() != config.auto_paste
            || self.screenshot.get() != config.screenshot
            || self.root.read(cx).shortcuts != config.shortcuts;
        self.status(cx, if changed { "Unsaved" } else { "Ready" }.into());
    }

    fn model_key(&self, cx: &App) -> String {
        let key = self.key.read(cx).value().trim();
        if key.is_empty() {
            std::env::var("MODEL_API_KEY").unwrap_or_default()
        } else {
            key.to_owned()
        }
    }

    fn close_stale_browser(&mut self, cx: &mut App) {
        if self
            .browser_key
            .as_deref()
            .is_some_and(|key| key != self.model_key(cx))
        {
            if let Some(handle) = self.browser.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            self.browser_key = None;
        }
    }

    fn end_capture(&mut self, cx: &mut App) -> bool {
        if !self.capturing {
            return true;
        }
        self.capturing = false;
        self.root.update(cx, |root, cx| {
            root.recording = None;
            cx.notify();
        });
        let mut failed = false;
        for hotkey in &mut self.hotkeys {
            if hotkey.is_some_and(|key| self.manager.register(key).is_err()) {
                *hotkey = None;
                failed = true;
            }
        }
        if failed {
            self.status(cx, "Could not restore a shortcut".into());
        }
        !failed
    }

    fn save(&mut self, next: &Config, path: &std::path::PathBuf) -> Result<(), String> {
        let keys = [
            next.to_hotkey(&next.shortcuts[0])?,
            next.to_hotkey(&next.shortcuts[1])?,
            next.to_hotkey(&next.shortcuts[2])?,
        ];
        if keys[0] == keys[1] || keys[0] == keys[2] || keys[1] == keys[2] {
            return Err("Choose different shortcuts".into());
        }
        let mut added = Vec::new();
        for key in keys {
            if !self.hotkeys.contains(&Some(key)) {
                if self.manager.register(key).is_err() {
                    for key in added {
                        let _ = self.manager.unregister(key);
                    }
                    return Err("Shortcut is already in use".into());
                }
                added.push(key);
            }
        }
        let mut removed = Vec::new();
        for key in self.hotkeys.iter().flatten() {
            if !keys.contains(key) {
                if let Err(error) = self.manager.unregister(*key) {
                    for key in removed {
                        let _ = self.manager.register(key);
                    }
                    for key in added {
                        let _ = self.manager.unregister(key);
                    }
                    return Err(error.to_string());
                }
                removed.push(*key);
            }
        }
        if let Err(error) = next.save(path) {
            for key in removed {
                let _ = self.manager.register(key);
            }
            for key in added {
                let _ = self.manager.unregister(key);
            }
            return Err(error);
        }
        self.hotkeys = keys.map(Some);
        Ok(())
    }

    fn browse(
        &mut self,
        cx: &mut App,
        kind: ModelKind,
        key: String,
        models: Vec<String>,
        tx: mpsc::Sender<Event>,
    ) {
        if let Some(handle) = self.browser.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        self.browser_key = None;
        let theme = self.theme.clone();
        let logo = self.logo.clone();
        let light_logo = self.light_logo.clone();
        let bounds = Bounds::centered(None, size(px(460.), px(360.)), cx);
        let (models, others) = models.into_iter().partition(|model| kind.supports(model));
        match cx.open_window(
            window_options(bounds, size(px(360.), px(240.)), "Models"),
            move |window, cx| {
                observed(
                    window,
                    cx,
                    ModelBrowser {
                        models,
                        others,
                        others_open: false,
                        kind,
                        tx,
                        theme,
                        logo,
                        light_logo,
                    },
                )
            },
        ) {
            Ok(handle) => {
                self.browser = Some(handle);
                self.browser_key = Some(key);
            }
            Err(e) => self.status(cx, format!("Could not open models: {e}")),
        }
    }

    fn browse_mics(&mut self, cx: &mut App, mics: Vec<String>, tx: mpsc::Sender<Event>) {
        if let Some(handle) = self.mic_browser.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        let theme = self.theme.clone();
        let logo = self.logo.clone();
        let light_logo = self.light_logo.clone();
        let bounds = Bounds::centered(None, size(px(460.), px(320.)), cx);
        match cx.open_window(
            window_options(bounds, size(px(360.), px(240.)), "Microphones"),
            move |window, cx| {
                observed(
                    window,
                    cx,
                    MicBrowser {
                        mics,
                        tx,
                        theme,
                        logo,
                        light_logo,
                    },
                )
            },
        ) {
            Ok(handle) => self.mic_browser = Some(handle),
            Err(e) => self.status(cx, format!("Could not open microphones: {e}")),
        }
    }

    fn indicator(&mut self, cx: &mut App, status: &str) {
        log_event(&format!("indicator: show {status:?}"));
        if let Some(handle) = &self.indicator {
            match handle.update(cx, |indicator, _, cx| {
                indicator.status = status.to_owned().into();
                cx.notify();
            }) {
                Ok(_) => {
                    log_event("indicator: updated existing window");
                    return;
                }
                Err(e) => log_event(&format!(
                    "indicator: existing window gone ({e:?}), recreating"
                )),
            }
        }
        let Some(display) = cx.primary_display() else {
            log_event("indicator: no primary display");
            return;
        };
        let screen = display.bounds();
        let bounds = Bounds::new(
            point(screen.right() - px(284.), screen.bottom() - px(108.)),
            size(px(260.), px(36.)),
        );
        log_event(&format!("indicator: screen={screen:?} window={bounds:?}"));
        let theme = self.applied_theme.clone();
        let icon = self.icon.clone();
        let icon_light = self.icon_light.clone();
        let status = status.to_owned();
        let handle = match cx.open_window(popup_options(bounds), move |window, cx| {
            observed(
                window,
                cx,
                Indicator {
                    status: status.into(),
                    theme,
                    icon,
                    icon_light,
                },
            )
        }) {
            Ok(handle) => handle,
            Err(e) => {
                log_event(&format!("indicator: open_window failed ({e:?})"));
                return;
            }
        };
        #[cfg(windows)]
        place_popup(&handle, cx, bounds, "indicator");
        self.indicator = Some(handle);
        log_event("indicator: window ready");
    }

    fn dismiss(&mut self, cx: &mut App) {
        if let Some(handle) = self.indicator.take() {
            log_event("indicator: dismiss");
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn answer_card(&mut self, cx: &mut App, text: String) {
        self.card_seq += 1;
        let seq = self.card_seq;
        let tx = self.tx.clone();
        if let Some(handle) = &self.card
            && handle
                .update(cx, |card, _, cx| {
                    card.text = text.clone().into();
                    card.seq = seq;
                    cx.notify();
                })
                .is_ok()
        {
            log_event("answer: updated existing card");
            return;
        }
        self.card = None;
        let Some(display) = cx.primary_display() else {
            log_event("answer: no primary display");
            return;
        };
        let screen = display.bounds();
        let bounds = Bounds::new(
            point(screen.right() - px(360.), screen.bottom() - px(160.)),
            size(px(340.), px(140.)),
        );
        let theme = self.theme.clone();
        let text: SharedString = text.into();
        let Ok(handle) = cx.open_window(popup_options(bounds), move |window, cx| {
            observed(
                window,
                cx,
                AnswerCard {
                    text,
                    theme,
                    tx,
                    seq,
                },
            )
        }) else {
            log_event("answer: open_window failed");
            return;
        };
        #[cfg(windows)]
        place_popup(&handle, cx, bounds, "answer");
        self.card = Some(handle);
        log_event("answer: card ready");
    }

    fn error(&mut self, cx: &mut App, config: &Config, message: String) {
        log_event(&format!("error: {message:?}"));
        self.dismiss(cx);
        self.refresh_status(cx, config);
        if let Some(handle) = &self.error {
            if handle
                .update(cx, |dialog, window, cx| {
                    dialog.message = message.clone().into();
                    window.activate_window();
                    cx.notify();
                })
                .is_ok()
            {
                log_event("error: updated existing dialog");
                return;
            }
            log_event("error: existing dialog gone, recreating");
        }
        self.error = None;
        let theme = self.theme.clone();
        let logo = self.logo.clone();
        let light_logo = self.light_logo.clone();
        let tx = self.tx.clone();
        let message_shared: SharedString = message.clone().into();
        let bounds = Bounds::centered(None, size(px(480.), px(230.)), cx);
        match cx.open_window(
            window_options(bounds, size(px(380.), px(180.)), "Error"),
            move |window, cx| {
                observed(
                    window,
                    cx,
                    ErrorDialog {
                        message: message_shared,
                        tx,
                        theme,
                        logo,
                        light_logo,
                    },
                )
            },
        ) {
            Ok(handle) => {
                log_event("error: dialog ready");
                self.error = Some(handle);
            }
            Err(e) => {
                log_event(&format!("error: open_window failed ({e:?})"));
                self.status(cx, message);
            }
        }
    }
}

#[cfg(windows)]
fn place_popup<V>(
    handle: &gpui::WindowHandle<V>,
    cx: &mut App,
    bounds: Bounds<gpui::Pixels>,
    name: &str,
) {
    let _ = handle.update(cx, |_, window, _| {
		use raw_window_handle::{HasWindowHandle, RawWindowHandle};
		use windows_sys::Win32::Foundation::{POINT, RECT};
		use windows_sys::Win32::UI::WindowsAndMessaging::*;
		if let Ok(raw) = HasWindowHandle::window_handle(window) {
			if let RawWindowHandle::Win32(raw) = raw.as_raw() {
				let hwnd = raw.hwnd.get() as *mut std::ffi::c_void;
				// placement only takes effect via SetWindowPlacement
				let scale = window.scale_factor();
				let x = (f32::from(bounds.origin.x) * scale) as i32;
				let y = (f32::from(bounds.origin.y) * scale) as i32;
				let w = (f32::from(bounds.size.width) * scale) as i32;
				let h = (f32::from(bounds.size.height) * scale) as i32;
				unsafe {
					let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
					SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE as isize);
					let placement = WINDOWPLACEMENT {
						length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
						flags: 0,
						showCmd: SW_SHOWNOACTIVATE as u32,
						ptMinPosition: POINT { x: -1, y: -1 },
						ptMaxPosition: POINT { x: -1, y: -1 },
						rcNormalPosition: RECT {
							left: x,
							top: y,
							right: x + w,
							bottom: y + h,
						},
					};
					let placed = SetWindowPlacement(hwnd, &placement);
					let topmost = SetWindowPos(
						hwnd,
						HWND_TOPMOST,
						0,
						0,
						0,
						0,
						SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
					);
					log_event(&format!(
						"{name}: win32 hwnd={hwnd:?} scale={scale} rect={x},{y},{w},{h} placed={placed} topmost={topmost}"
					));
				}
			} else {
				log_event(&format!("{name}: win32 handle is not Win32"));
			}
		} else {
			log_event(&format!("{name}: no win32 window handle"));
		}
	});
}

fn main() {
    let config_path = Config::path();
    let mut config = Config::load(&config_path).unwrap_or_else(|err| {
        eprintln!("config error: {err}");
        std::process::exit(1);
    });
    let (tx, rx) = mpsc::channel::<Event>();
    Application::new().run(move |cx: &mut App| {
        // need to share gpui's os message pump
        let manager = GlobalHotKeyManager::new().expect("hotkey manager");
        let mut hotkeys = [None; 3];
        let mut errors = Vec::new();
        for (i, trigger) in config.shortcuts.iter().enumerate() {
            match config.to_hotkey(trigger) {
                Ok(hotkey) => {
                    if manager.register(hotkey).is_ok() {
                        hotkeys[i] = Some(hotkey);
                    } else {
                        errors.push(format!("{} shortcut is already in use", trigger.name));
                    }
                }
                Err(err) => errors.push(format!("{} shortcut: {err}", trigger.name)),
            }
        }
        GlobalHotKeyEvent::set_event_handler(Some({
            let tx = tx.clone();
            move |event: GlobalHotKeyEvent| {
                if event.state == HotKeyState::Pressed {
                    let _ = tx.send(Event::Trigger(event.id));
                }
            }
        }));
        std::mem::forget(build_tray());
        MenuEvent::set_event_handler(Some({
            let tx = tx.clone();
            move |event: MenuEvent| {
                let next = match event.id.0.as_str() {
                    "show" => Event::Show,
                    "quit" => Event::Quit,
                    _ => return,
                };
                let _ = tx.send(next);
            }
        }));
        cx.bind_keys([
            KeyBinding::new("backspace", field::Backspace, Some("field")),
            KeyBinding::new("delete", field::Delete, Some("field")),
            KeyBinding::new("left", field::Left, Some("field")),
            KeyBinding::new("right", field::Right, Some("field")),
            KeyBinding::new("home", field::Home, Some("field")),
            KeyBinding::new("end", field::End, Some("field")),
            KeyBinding::new("ctrl-v", field::Paste, Some("field")),
            KeyBinding::new("cmd-v", field::Paste, Some("field")),
        ]);
        let logo = render_image(include_bytes!("../assets/logo.png"));
        let light_logo = render_image(include_bytes!("../assets/logo-light.png"));
        let icon = render_image(include_bytes!("../assets/icon.png"));
        let icon_light = render_image(include_bytes!("../assets/icon-light.png"));
        let theme = Rc::new(Cell::new(config.theme));
        let applied_theme = Rc::new(Cell::new(config.theme));
        let key = cx.new(|cx| {
            Field::new(
                &config.api_key,
                "MODEL_API_KEY",
                true,
                true,
                tx.clone(),
                theme.clone(),
                cx,
            )
        });
        let voice = cx.new(|cx| {
            Field::new(
                &config.voice_model,
                VOICE_MODEL,
                false,
                true,
                tx.clone(),
                theme.clone(),
                cx,
            )
        });
        let answer = cx.new(|cx| {
            Field::new(
                &config.model,
                MUSE_MODEL,
                false,
                true,
                tx.clone(),
                theme.clone(),
                cx,
            )
        });
        let mic = cx.new(|cx| {
            Field::new(
                &config.mic,
                "Default input",
                false,
                false,
                tx.clone(),
                theme.clone(),
                cx,
            )
        });
        let auto_paste = Rc::new(Cell::new(config.auto_paste));
        let screenshot = Rc::new(Cell::new(config.screenshot));
        let root = cx.new(|cx| {
            Root::new(
                tx.clone(),
                config.shortcuts.clone(),
                cx.focus_handle(),
                theme.clone(),
                auto_paste.clone(),
                screenshot.clone(),
                key.clone(),
                voice.clone(),
                answer.clone(),
                mic.clone(),
                logo.clone(),
                light_logo.clone(),
            )
        });
        let mut settings = Settings {
            tx: tx.clone(),
            root,
            window: None,
            key,
            voice,
            answer,
            mic,
            theme,
            applied_theme,
            auto_paste,
            screenshot,
            model_request: 0,
            model_cache: HashMap::new(),
            indicator: None,
            browser: None,
            browser_key: None,
            mic_browser: None,
            error: None,
            card: None,
            card_seq: 0,
            icon,
            icon_light,
            logo,
            light_logo,
            manager,
            hotkeys,
            capturing: false,
        };
        settings.show(cx);
        if !errors.is_empty() {
            settings.status(cx, errors.join(". "));
        }
        let poll_tx = tx.clone();
        let mut state = State::Idle;
        cx.spawn(async move |cx| {
            loop {
                let mut end = false;
                let _ = cx.update(|cx: &mut App| {
                    loop {
                        match rx.try_recv() {
                            Ok(event) => handle(
                                event,
                                cx,
                                &mut settings,
                                &mut config,
                                &config_path,
                                &poll_tx,
                                &mut state,
                            ),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                end = true;
                                break;
                            }
                        }
                    }
                });
                if end {
                    break;
                }
                Timer::after(Duration::from_millis(80)).await;
            }
        })
        .detach();
    });
}

#[cfg(windows)]
fn show_settings(window: &gpui::Window, show: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, ShowWindow};
    if let Ok(raw) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(raw) = raw.as_raw()
    {
        unsafe {
            ShowWindow(
                raw.hwnd.get() as *mut std::ffi::c_void,
                if show { SW_SHOW } else { SW_HIDE },
            );
        }
    }
}

enum State {
    Idle,
    Recording {
        index: usize,
        stop: mpsc::Sender<()>,
    },
    Processing,
}

#[allow(clippy::too_many_arguments)]
fn handle(
    event: Event,
    cx: &mut App,
    settings: &mut Settings,
    config: &mut Config,
    config_path: &std::path::PathBuf,
    tx: &mpsc::Sender<Event>,
    state: &mut State,
) {
    match event {
        Event::Trigger(id) => {
            log_event(&format!("trigger: id={id}"));
            if !settings.capturing
                && let Some(index) = settings
                    .hotkeys
                    .iter()
                    .position(|key| key.is_some_and(|key| key.id() == id))
            {
                match state {
                    State::Idle => {
                        let key = config.effective_key();
                        if key.is_empty() {
                            settings.error(cx, config, "Set an API key in settings".into());
                            return;
                        }
                        let target = paste::foreground();
                        let model = config.model.clone();
                        let voice_model = config.voice_model.clone();
                        let mic = config.mic.clone();
                        let with_shot = index != 0 && config.screenshot;
                        let label = config.shortcuts[index].name.clone();
                        let (stop, rx) = mpsc::channel();
                        *state = State::Recording { index, stop };
                        log_event(&format!("record: start index={index}"));
                        settings.indicator(
                            cx,
                            if index == 0 {
                                "Recording dictation"
                            } else if index == 1 {
                                "Recording question"
                            } else {
                                "Recording helper"
                            },
                        );
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let result = audio::record_wav(rx, &mic).and_then(|wav| {
                                let _ = tx.send(Event::Stage("Transcribing"));
                                let text =
                                    muse::transcribe(MUSE_BASE_URL, &key, &voice_model, &wav)?;
                                if index == 0 {
                                    Ok(text)
                                } else {
                                    let _ = tx.send(Event::Stage("Answering"));
                                    let shot = if with_shot {
                                        match capture::screenshot_png() {
                                            Ok(png) => Some(png),
                                            Err(e) => {
                                                log_event(&format!("screenshot failed: {e}"));
                                                None
                                            }
                                        }
                                    } else {
                                        None
                                    };
                                    muse::ask(MUSE_BASE_URL, &key, &model, &text, shot.as_deref())
                                }
                            });
                            let _ = tx.send(Event::Done {
                                label,
                                target,
                                card: index == 2,
                                result,
                            });
                        });
                    }
                    State::Recording {
                        index: active,
                        stop,
                    } if *active == index => {
                        let _ = stop.send(());
                        *state = State::Processing;
                        log_event("record: stop requested");
                        settings.indicator(cx, "Processing");
                    }
                    _ => settings.status(cx, "Finish the current recording first".into()),
                }
            }
        }
        Event::Show => settings.show(cx),
        Event::FieldsChanged => {
            settings.close_stale_browser(cx);
            settings.refresh_status(cx, config);
        }
        Event::ClearKey => {
            settings
                .key
                .update(cx, |field, cx| field.set(String::new(), cx));
            settings.close_stale_browser(cx);
            settings.refresh_status(cx, config);
        }
        Event::CaptureStart(index) => {
            if !matches!(state, State::Idle) {
                settings.status(cx, "Finish the current recording first".into());
                return;
            }
            if !settings.capturing {
                let mut removed = Vec::new();
                for key in settings.hotkeys.iter().flatten() {
                    if let Err(error) = settings.manager.unregister(*key) {
                        for key in removed {
                            let _ = settings.manager.register(key);
                        }
                        settings.status(cx, format!("Could not record shortcut: {error}"));
                        return;
                    }
                    removed.push(*key);
                }
                settings.capturing = true;
            }
            if let Some(window) = &settings.window {
                let _ = window.update(cx, |root, window, cx| {
                    root.recording = Some(index);
                    root.shortcut_modifiers = gpui::Modifiers::default();
                    window.focus(&root.shortcut_focus);
                    cx.notify();
                });
            }
        }
        Event::CaptureKey(stroke) => {
            let index = settings.root.read(cx).recording;
            if index.is_none() {
                return;
            }
            if stroke.key == "escape" {
                if settings.end_capture(cx) {
                    settings.refresh_status(cx, config);
                }
            } else if let Some(index) = index {
                let name = settings.root.read(cx).shortcuts[index].name.clone();
                if let Some(shortcut) = config::capture_hotkey(&name, &stroke) {
                    settings.root.update(cx, |root, cx| {
                        root.shortcuts[index] = shortcut;
                        cx.notify();
                    });
                    if settings.end_capture(cx) {
                        settings.refresh_status(cx, config);
                    }
                } else {
                    settings.status(cx, "Use a letter, number, F key, or Space".into());
                }
            }
        }
        Event::CaptureEnd => {
            if settings.end_capture(cx) {
                settings.refresh_status(cx, config);
            }
        }
        Event::Stage(status) => {
            log_event(&format!("stage: {status}"));
            settings.indicator(cx, status);
        }
        Event::SetTheme(theme) => {
            settings.theme.set(theme);
            settings.refresh_status(cx, config);
            for field in [
                &settings.key,
                &settings.voice,
                &settings.answer,
                &settings.mic,
            ] {
                field.update(cx, |_, cx| cx.notify());
            }
            if let Some(handle) = &settings.browser {
                let _ = handle.update(cx, |_, _, cx| cx.notify());
            }
            if let Some(handle) = &settings.mic_browser {
                let _ = handle.update(cx, |_, _, cx| cx.notify());
            }
        }
        Event::SetAutoPaste(on) => {
            settings.auto_paste.set(on);
            settings.refresh_status(cx, config);
        }
        Event::SetScreenshot(on) => {
            settings.screenshot.set(on);
            settings.refresh_status(cx, config);
        }
        Event::SaveProvider => {
            if !settings.end_capture(cx) {
                return;
            }
            let mut next = config.clone();
            next.api_key = settings.key.read(cx).value().trim().to_string();
            next.voice_model = settings.voice.read(cx).value().trim().to_string();
            next.model = settings.answer.read(cx).value().trim().to_string();
            next.mic = settings.mic.read(cx).value().trim().to_string();
            next.theme = settings.theme.get();
            next.auto_paste = settings.auto_paste.get();
            next.screenshot = settings.screenshot.get();
            next.shortcuts = settings.root.read(cx).shortcuts.clone();
            if !matches!(state, State::Idle) && next.shortcuts != config.shortcuts {
                settings.status(cx, "Finish the current recording first".into());
                return;
            }
            if next.voice_model.is_empty() || next.model.is_empty() {
                settings.error(cx, config, "Select both models".into());
            } else {
                match settings.save(&next, config_path) {
                    Ok(()) => {
                        *config = next;
                        settings.applied_theme.set(config.theme);
                        if let Some(handle) = &settings.indicator {
                            let _ = handle.update(cx, |_, _, cx| cx.notify());
                        }
                        settings.refresh_status(cx, config);
                    }
                    Err(e) => settings.error(cx, config, format!("Save failed: {e}")),
                }
            }
        }
        Event::CancelSettings => {
            let restored = settings.end_capture(cx);
            settings.model_request += 1;
            for (field, value) in [
                (&settings.key, &config.api_key),
                (&settings.voice, &config.voice_model),
                (&settings.answer, &config.model),
                (&settings.mic, &config.mic),
            ] {
                field.update(cx, |field, cx| field.set(value.clone(), cx));
            }
            settings.theme.set(config.theme);
            settings.auto_paste.set(config.auto_paste);
            settings.screenshot.set(config.screenshot);
            settings.root.update(cx, |root, cx| {
                root.shortcuts = config.shortcuts.clone();
                cx.notify();
            });
            if restored {
                settings.refresh_status(cx, config);
            }
            if let Some(handle) = settings.browser.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            settings.browser_key = None;
            if let Some(handle) = settings.mic_browser.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            if let Some(handle) = &settings.window {
                let _ = handle.update(cx, |_, window, _| {
                    #[cfg(windows)]
                    show_settings(window, false);
                    #[cfg(not(windows))]
                    window.remove_window();
                });
            }
        }
        Event::RefreshModels(kind) => {
            let key = settings.model_key(cx);
            if key.is_empty() {
                settings.error(cx, config, "Enter a Meta API key first".into());
                return;
            }
            settings.model_request += 1;
            if let Some(models) = settings.model_cache.get(&key).cloned() {
                settings.refresh_status(cx, config);
                settings.browse(cx, kind, key, models, tx.clone());
                return;
            }
            settings.status(cx, "Loading models".into());
            let request = settings.model_request;
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = muse::models(&key);
                let _ = tx.send(Event::Models(request, kind, key, result));
            });
        }
        Event::Models(request, kind, key, result) => {
            if request == settings.model_request && key == settings.model_key(cx) {
                match result {
                    Ok(models) => {
                        settings.model_cache.insert(key.clone(), models.clone());
                        settings.refresh_status(cx, config);
                        settings.browse(cx, kind, key, models, tx.clone());
                    }
                    Err(e) => settings.error(cx, config, format!("Could not load models: {e}")),
                }
            }
        }
        Event::SelectModel(kind, model) => {
            settings.close_stale_browser(cx);
            if settings.browser_key.is_none() {
                return;
            }
            let field = match kind {
                ModelKind::Voice => &settings.voice,
                ModelKind::Answer => &settings.answer,
            };
            field.update(cx, |f, cx| f.set(model, cx));
            if let Some(handle) = settings.browser.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            settings.browser_key = None;
            settings.refresh_status(cx, config);
        }
        Event::RefreshMics => {
            let mics = audio::input_devices();
            if mics.is_empty() {
                settings.error(cx, config, "No microphones found".into());
                return;
            }
            settings.browse_mics(cx, mics, tx.clone());
        }
        Event::SelectMic(mic) => {
            settings.mic.update(cx, |f, cx| f.set(mic, cx));
            if let Some(handle) = settings.mic_browser.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            settings.refresh_status(cx, config);
        }
        Event::Done {
            label,
            target,
            card,
            result,
        } => match result {
            Ok(text) => {
                log_event(&format!("done: ok chars={}", text.len()));
                if card {
                    *state = State::Idle;
                    settings.dismiss(cx);
                    settings.answer_card(cx, text);
                    let seq = settings.card_seq;
                    let tx2 = tx.clone();
                    cx.spawn(async move |_cx| {
                        Timer::after(Duration::from_secs(10)).await;
                        let _ = tx2.send(Event::DismissAnswer(seq));
                    })
                    .detach();
                    return;
                }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                if cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    != Some(text.as_str())
                {
                    *state = State::Idle;
                    settings.error(cx, config, "Clipboard write failed".into());
                    return;
                }
                if !config.auto_paste {
                    *state = State::Idle;
                    settings.dismiss(cx);
                    settings.refresh_status(cx, config);
                    return;
                }
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Event::Pasted(paste::paste(target)));
                });
            }
            Err(e) => {
                *state = State::Idle;
                settings.error(cx, config, format!("{label}: {e}"));
            }
        },
        Event::Pasted(result) => {
            *state = State::Idle;
            match result {
                Ok(()) => {
                    log_event("pasted: ok");
                    settings.dismiss(cx);
                    settings.refresh_status(cx, config);
                }
                Err(e) => settings.error(cx, config, format!("Copied. {e}")),
            }
        }
        Event::DismissAnswer(seq) => {
            if seq == settings.card_seq
                && let Some(handle) = settings.card.take()
            {
                log_event("answer: dismissed");
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        Event::DismissError => {
            if let Some(handle) = settings.error.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        Event::Quit => cx.quit(),
    }
}

#[cfg(windows)]
mod paste {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::*, WindowsAndMessaging::GetForegroundWindow,
    };
    pub fn foreground() -> isize {
        unsafe { GetForegroundWindow() as isize }
    }
    pub fn paste(target: isize) -> Result<(), String> {
        for _ in 0..40 {
            if unsafe {
                [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
                    .iter()
                    .all(|&k| GetAsyncKeyState(k as i32) >= 0)
            } {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if target == 0 || foreground() != target {
            return Err("Focus changed. Please paste manually.".into());
        }
        if unsafe {
            [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
                .iter()
                .any(|&k| GetAsyncKeyState(k as i32) < 0)
        } {
            return Err("Release shortcut keys and paste manually.".into());
        }
        let key = |vk, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [
            key(VK_CONTROL, 0),
            key(b'V' as u16, 0),
            key(b'V' as u16, KEYEVENTF_KEYUP),
            key(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        if unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        } != inputs.len() as u32
        {
            return Err("Automatic paste failed. Please paste manually.".into());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod paste {
    pub fn foreground() -> isize {
        0
    }

    #[cfg(target_os = "macos")]
    fn hint(error: String) -> String {
        format!(
            "Could not simulate paste. Grant Accessibility permission, then paste manually: {error}"
        )
    }

    #[cfg(not(target_os = "macos"))]
    fn hint(error: String) -> String {
        format!("Could not simulate paste. X11 is required; on Wayland paste manually: {error}")
    }

    pub fn paste(_: isize) -> Result<(), String> {
        use enigo::{
            Direction::{Click, Press, Release},
            Enigo, Key, Keyboard, Settings as EnigoSettings,
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut enigo =
            Enigo::new(&EnigoSettings::default()).map_err(|e| hint(format!("{e:?}")))?;
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;
        enigo
            .key(modifier, Press)
            .map_err(|e| format!("press paste modifier: {e:?}"))?;
        let typed = enigo
            .key(Key::Unicode('v'), Click)
            .map_err(|e| format!("type paste key: {e:?}"));
        let _ = enigo.key(modifier, Release);
        typed
    }
}

fn build_tray() -> tray_icon::TrayIcon {
    let rgba = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("tray icon")
        .thumbnail(64, 64)
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    let icon = Icon::from_rgba(rgba.into_raw(), w, h).expect("tray icon");
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("show", "Show", true, None))
        .expect("menu");
    menu.append(&MenuItem::with_id("quit", "Quit", true, None))
        .expect("menu");
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("trigger")
        .with_icon(icon)
        .build()
        .expect("tray")
}
