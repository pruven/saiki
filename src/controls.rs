use crate::capture::{self, CaptureError, Keyboard};
use crate::overlay::{self, EDGE, MUTED, Overlay, SURFACE, TEXT};
use crate::position::Position;
use gpui::prelude::*;
use gpui::{
    App, Bounds, ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, KeyBinding,
    KeyDownEvent, MouseButton, Task, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions, actions, div, px, rgb, rgba, size,
};
use serde_json::Value;
use std::{env, fs};

actions!(saiki, [ToggleCapture, ShowHelp, ChoosePosition, Quit]);

pub struct Controls {
    focus: FocusHandle,
    layer_shell: bool,
    capture: Option<Task<()>>,
    capturing: bool,
    shortcuts_only: bool,
    help: bool,
    error: Option<CaptureError>,
    overlay: Option<WindowHandle<Overlay>>,
    position: Position,
    pending_position: Option<Position>,
    translations: Value,
    tray_status: async_channel::Sender<&'static str>,
}

impl Controls {
    pub fn new(
        cx: &mut App,
        layer_shell: bool,
        tray_status: async_channel::Sender<&'static str>,
    ) -> Entity<Self> {
        cx.bind_keys([
            KeyBinding::new("space", ToggleCapture, Some("Saiki")),
            KeyBinding::new("shift-/", ShowHelp, Some("Saiki")),
            KeyBinding::new("p", ChoosePosition, Some("Saiki")),
            KeyBinding::new("q", Quit, Some("Saiki")),
        ]);
        cx.new(|cx| Self {
            focus: cx.focus_handle(),
            layer_shell,
            capture: None,
            capturing: false,
            shortcuts_only: false,
            help: false,
            error: (!layer_shell).then_some(CaptureError::Overlay),
            overlay: None,
            position: Position::load(),
            pending_position: None,
            translations: translations(),
            tray_status,
        })
    }

    pub fn show(cx: &mut App, state: &Entity<Self>) -> anyhow::Result<WindowHandle<Self>> {
        let focus = state.read(cx).focus.clone();
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Saiki".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(440.), px(375.)),
                    cx,
                ))),
                window_min_size: Some(size(px(360.), px(360.))),
                app_id: Some("saiki".into()),
                ..Default::default()
            },
            |window, cx| {
                focus.focus(window, cx);
                state.clone()
            },
        )
    }

    pub fn text(&self, english: &'static str) -> String {
        self.translations[english]
            .as_str()
            .unwrap_or(english)
            .to_owned()
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.capture.is_some() {
            self.stop(cx);
            return;
        }
        self.start(cx);
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if !self.layer_shell || self.capture.is_some() {
            return;
        }
        self.error = None;
        self.capture = Some(cx.spawn(async move |this, cx| {
            let result = capture::while_unlocked(async {
                let mut keyboard = Keyboard::open().await?;
                this.update(cx, |this, cx| {
                    this.capturing = true;
                    let _ = this.tray_status.force_send("Capturing keys");
                    cx.notify();
                })
                .map_err(|_| CaptureError::Device)?;

                loop {
                    let keys = keyboard.next_strokes().await?;
                    if !keys.is_empty() {
                        this.update(cx, |this, cx| {
                            let strokes = keys
                                .into_iter()
                                .filter(|key| !this.shortcuts_only || key.is_shortcut());
                            overlay::show(cx, &mut this.overlay, this.position, strokes)
                                .map_err(|_| CaptureError::Overlay)
                        })
                        .map_err(|_| CaptureError::Device)??;
                    }
                }
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(task) = this.capture.take() {
                    task.detach();
                }
                this.error = result.err();
                this.stop(cx);
                if let Some(error) = this.error {
                    let _ = this.tray_status.force_send(error.message());
                }
            });
        }));
        let _ = self.tray_status.force_send("Opening keyboard…");
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.capture = None;
        self.capturing = false;
        let _ = self.tray_status.force_send("Capture paused");
        self.clear_overlay(cx);
        cx.notify();
    }

    fn clear_overlay(&mut self, cx: &mut Context<Self>) {
        if let Some(overlay) = self.overlay.take() {
            let _ = overlay.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn preview(&mut self, cx: &mut Context<Self>) {
        if self.layer_shell
            && overlay::show(cx, &mut self.overlay, self.position, overlay::demo()).is_err()
        {
            self.error = Some(CaptureError::Overlay);
            cx.notify();
        }
    }

    fn toggle_position_picker(&mut self, cx: &mut Context<Self>) {
        self.pending_position = if self.pending_position.is_some() {
            None
        } else {
            self.help = false;
            Some(self.position)
        };
        cx.notify();
    }

    fn pick_position(&mut self, position: Position, cx: &mut Context<Self>) {
        self.pending_position = None;
        if self.position != position {
            self.position = position;
            if position.save().is_err() {
                self.error = Some(CaptureError::Position);
            } else if self.error == Some(CaptureError::Position) {
                self.error = None;
            }
            if overlay::relocate(cx, &mut self.overlay, position).is_err() {
                self.error = Some(CaptureError::Overlay);
            }
        }
        cx.notify();
    }

    fn position_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.pending_position else {
            return;
        };
        let index = Position::ALL
            .iter()
            .position(|position| *position == selected)
            .unwrap();
        let target = match event.keystroke.key.as_str() {
            "left" => index / 3 * 3 + (index % 3).saturating_sub(1),
            "right" => index / 3 * 3 + (index % 3 + 1).min(2),
            "up" => index % 3,
            "down" => 3 + index % 3,
            "enter" => return self.pick_position(selected, cx),
            "escape" => return self.toggle_position_picker(cx),
            _ => return,
        };
        self.pending_position = Some(Position::ALL[target]);
        cx.notify();
    }

    fn position_picker(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .child(self.text("Overlay position"))
                    .child(
                        button("position", self.text(self.position.label()), false).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toggle_position_picker(cx)
                            }),
                        ),
                    ),
            )
            .when_some(self.pending_position, |element, selected| {
                element.children(Position::ALL.chunks(3).map(|row| {
                    div()
                        .flex()
                        .gap(px(5.))
                        .children(row.iter().map(|&position| {
                            button(
                                position.label(),
                                self.text(position.label()),
                                selected == position,
                            )
                            .flex_1()
                            .justify_center()
                            .px(px(5.))
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.pick_position(position, cx);
                                },
                            ))
                        }))
                }))
            })
    }
}

impl Render for Controls {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = if self.capturing {
            "Capturing keys"
        } else if self.capture.is_some() {
            "Opening keyboard…"
        } else {
            "Capture paused"
        };
        let detail = if self.help {
            "Space  Start or pause capture\nP  Choose overlay position\nQ  Quit\n?  Show keyboard shortcuts"
        } else if let Some(error) = self.error {
            error.message()
        } else if self.capturing {
            "Capture continues after you close this window."
        } else {
            "Show keypresses over the other apps on your screen."
        };

        div()
            .track_focus(&self.focus)
            .key_context("Saiki")
            .on_action(cx.listener(|this, _: &ToggleCapture, _, cx| this.toggle(cx)))
            .on_action(
                cx.listener(|this, _: &ChoosePosition, _, cx| this.toggle_position_picker(cx)),
            )
            .on_key_down(cx.listener(Self::position_key))
            .on_action(cx.listener(|this, _: &ShowHelp, _, cx| {
                this.pending_position = None;
                this.help = !this.help;
                cx.notify();
            }))
            .on_action(|_: &Quit, _, cx| cx.quit())
            .size_full()
            .bg(rgb(0x1b1b1b))
            .text_color(rgb(TEXT))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div().w_full().max_w(px(480.)).px(px(20.)).child(
                    div()
                        .rounded(px(18.))
                        .bg(rgb(SURFACE))
                        .border_1()
                        .border_color(rgba(EDGE))
                        .p(px(17.))
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .on_mouse_down(MouseButton::Left, |_, window, _| {
                                            window.start_window_move()
                                        })
                                        .text_size(px(12.))
                                        .text_color(rgb(MUTED))
                                        .child(overlay::keyboard_mark())
                                        .child("Saiki"),
                                )
                                .child(chrome_button("minimize", "−", |_, window, _| {
                                    window.minimize_window()
                                }))
                                .child(chrome_button("close", "×", |_, window, _| {
                                    window.remove_window()
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .text_size(px(23.))
                                        .line_height(px(30.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(self.text(status)),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .line_height(px(18.))
                                        .text_color(rgb(MUTED))
                                        .children(
                                            self.text(detail)
                                                .lines()
                                                .map(|line| div().child(line.to_owned()))
                                                .collect::<Vec<_>>(),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .id("shortcuts-only")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.shortcuts_only = !this.shortcuts_only;
                                    if this.shortcuts_only {
                                        this.clear_overlay(cx);
                                    }
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(16.))
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(rgba(0xffffff39))
                                        .when(self.shortcuts_only, |el| {
                                            el.bg(rgb(0xdadbea))
                                                .text_color(rgb(0x1b1b1b))
                                                .child("✓")
                                        }),
                                )
                                .child(self.text("Only show Ctrl, Alt or Super shortcuts")),
                        )
                        .child(self.position_picker(cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .child(
                                    button(
                                        "capture",
                                        self.text(if self.capture.is_some() {
                                            "Pause capture"
                                        } else {
                                            "Start capture"
                                        }),
                                        true,
                                    )
                                    .on_click(
                                        cx.listener(|this, _: &ClickEvent, _, cx| this.toggle(cx)),
                                    ),
                                )
                                .child(button("preview", self.text("Preview"), false).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.preview(cx)),
                                )),
                        ),
                ),
            )
    }
}

fn button(id: &'static str, text: String, primary: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .items_center()
        .h(px(32.))
        .px(px(12.))
        .rounded(px(9.))
        .border_1()
        .border_color(rgba(EDGE))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .bg(rgb(if primary { 0xe2e2e5 } else { 0x353536 }))
        .text_color(rgb(if primary { 0x222224 } else { TEXT }))
        .hover(move |style| style.bg(rgb(if primary { 0xf5f5f5 } else { 0x404041 })))
        .child(text)
}

fn chrome_button(
    id: &'static str,
    label: &'static str,
    action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .size(px(23.))
        .rounded(px(7.))
        .text_color(rgb(MUTED))
        .hover(|style| style.bg(rgb(0x404041)))
        .child(label)
        .on_click(action)
}

fn translations() -> Value {
    let language = env::var_os("HOME")
        .and_then(|home| {
            fs::read_to_string(std::path::PathBuf::from(home).join(".config/omarchy/language")).ok()
        })
        .or_else(|| env::var("LANG").ok())
        .unwrap_or_default();
    if language.trim().split(['_', '-', '.']).next() == Some("da") {
        return serde_json::from_str(include_str!("i18n/da.json")).unwrap_or_default();
    }
    Value::Null
}
