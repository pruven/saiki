use crate::capture::{self, CaptureError, Keyboard};
use crate::overlay::{self, EDGE, MUTED, Overlay, SURFACE, TEXT};
use crate::settings::{FadeDelay, Position, SuperKeyStyle};
use crate::updates;
#[cfg(target_os = "linux")]
use gpui::MouseButton;
use gpui::prelude::*;
use gpui::{
    App, Bounds, ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, KeyBinding,
    KeyDownEvent, Task, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
    actions, div, px, rgb, rgba, size,
};
use serde_json::Value;
use std::{env, fs};

actions!(
    saiki,
    [
        ToggleCapture,
        ShowHelp,
        ChoosePosition,
        ShorterDelay,
        LongerDelay,
        Quit
    ]
);

pub struct Controls {
    focus: FocusHandle,
    layer_shell: bool,
    capture: Option<Task<()>>,
    capturing: bool,
    shortcuts_only: bool,
    help: bool,
    error: Option<CaptureError>,
    overlay: Option<WindowHandle<Overlay>>,
    placement_editor: Option<WindowHandle<overlay::PlacementEditor>>,
    position: Position,
    position_profiles: Vec<Position>,
    profile_draft: Option<Position>,
    fade_delay: FadeDelay,
    super_key_style: SuperKeyStyle,
    update: UpdateStatus,
    pending_position: Option<Position>,
    translations: Value,
    tray_status: async_channel::Sender<&'static str>,
}

enum UpdateStatus {
    Idle,
    Checking,
    Current,
    Available(updates::Release),
    Failed,
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
            KeyBinding::new("[", ShorterDelay, Some("Saiki")),
            KeyBinding::new("]", LongerDelay, Some("Saiki")),
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
            placement_editor: None,
            position: Position::load(),
            position_profiles: Position::load_profiles(),
            profile_draft: None,
            fade_delay: FadeDelay::load(),
            super_key_style: SuperKeyStyle::load(),
            update: UpdateStatus::Idle,
            pending_position: None,
            translations: translations(),
            tray_status,
        })
    }

    pub fn show(cx: &mut App, state: &Entity<Self>) -> anyhow::Result<WindowHandle<Self>> {
        let focus = state.read(cx).focus.clone();
        let height = if cfg!(target_os = "linux") {
            380.
        } else {
            420.
        };
        let window = cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Saiki".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(440.), px(height)),
                    cx,
                ))),
                #[cfg(target_os = "windows")]
                window_min_size: Some(size(px(360.), px(405.))),
                #[cfg(target_os = "linux")]
                is_resizable: false,
                app_id: Some("saiki".into()),
                ..Default::default()
            },
            |window, cx| {
                focus.focus(window, cx);
                state.clone()
            },
        )?;
        state.update(cx, |this, cx| {
            if matches!(this.update, UpdateStatus::Idle) {
                this.check_for_updates(cx);
            }
        });
        Ok(window)
    }

    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update, UpdateStatus::Checking) {
            return;
        }
        self.update = UpdateStatus::Checking;
        cx.notify();
        let check = cx.background_spawn(async { updates::check() });
        cx.spawn(async move |this, cx| {
            let status = match check.await {
                Ok(Some(release)) => UpdateStatus::Available(release),
                Ok(None) => UpdateStatus::Current,
                Err(error) => {
                    eprintln!("saiki: update check failed: {error:#}");
                    UpdateStatus::Failed
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.update = status;
                cx.notify();
            });
        })
        .detach();
    }

    fn update_control(&self, cx: &mut Context<Self>) -> Div {
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(7.))
            .text_size(px(13.))
            .text_color(rgb(MUTED))
            .when(!matches!(self.update, UpdateStatus::Checking), |row| {
                row.child(
                    button("check-for-updates", self.text("Check for updates"), false).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.check_for_updates(cx)),
                    ),
                )
            });
        match &self.update {
            UpdateStatus::Available(release) => {
                let page = release.page_url.clone();
                row = row
                    .child(format!(
                        "{} v{}",
                        self.text("Update available"),
                        release.version
                    ))
                    .child(
                        button("release-notes", self.text("Release notes"), false)
                            .on_click(move |_, _, cx| cx.open_url(&page)),
                    );
                if let Some(url) = release.download_url.clone() {
                    row = row.child(
                        button("download-update", self.text("Download update"), true)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                row
            }
            UpdateStatus::Checking => row.child(self.text("Checking for updates…")),
            UpdateStatus::Current => row.child(self.text("No update available")),
            UpdateStatus::Failed => row.child(self.text("Could not check for updates")),
            UpdateStatus::Idle => row,
        }
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
                            if this
                                .placement_editor
                                .is_some_and(|editor| editor.read(cx).is_ok())
                            {
                                return Ok(());
                            }
                            let strokes = keys
                                .into_iter()
                                .filter(|key| !this.shortcuts_only || key.is_shortcut());
                            overlay::show(
                                cx,
                                &mut this.overlay,
                                this.position,
                                this.fade_delay.duration(),
                                this.super_key_style,
                                strokes,
                            )
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
            && overlay::show(
                cx,
                &mut self.overlay,
                self.position,
                self.fade_delay.duration(),
                self.super_key_style,
                overlay::demo(),
            )
            .is_err()
        {
            self.error = Some(CaptureError::Overlay);
            cx.notify();
        }
    }

    fn toggle_position_picker(&mut self, cx: &mut Context<Self>) {
        self.profile_draft = None;
        self.pending_position = if self.pending_position.is_some() {
            None
        } else {
            self.help = false;
            Some(self.position)
        };
        cx.notify();
    }

    fn pick_position(&mut self, position: Position, cx: &mut Context<Self>) -> bool {
        if self.position != position {
            if position.save().is_err() {
                self.error = Some(CaptureError::Position);
                cx.notify();
                return false;
            }
            self.position = position;
            if self.error == Some(CaptureError::Position) {
                self.error = None;
            }
            if overlay::relocate(cx, &mut self.overlay, position).is_err() {
                self.error = Some(CaptureError::Overlay);
            }
        }
        self.pending_position = None;
        self.profile_draft = None;
        cx.notify();
        true
    }

    fn position_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.pending_position else {
            return;
        };
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                self.pick_position(selected, cx);
                return;
            }
            "escape" => return self.toggle_position_picker(cx),
            _ => {}
        }
        let index = Position::ALL
            .iter()
            .position(|position| *position == selected)
            .unwrap_or(2); // Start arrow navigation from the top-right preset.
        let target = match key {
            "left" => index / 3 * 3 + (index % 3).saturating_sub(1),
            "right" => index / 3 * 3 + (index % 3 + 1).min(2),
            "up" => index % 3,
            "down" => 3 + index % 3,
            _ => return,
        };
        self.pending_position = Some(Position::ALL[target]);
        cx.notify();
    }

    fn adjust_delay(&mut self, longer: bool, cx: &mut Context<Self>) {
        let delay = self.fade_delay.step(longer);
        if delay == self.fade_delay {
            return;
        }
        self.fade_delay = delay;
        if delay.save().is_err() {
            self.error = Some(CaptureError::FadeDelay);
        } else if self.error == Some(CaptureError::FadeDelay) {
            self.error = None;
        }
        if let Some(overlay) = self.overlay {
            let _ = overlay.update(cx, |view, _, cx| view.set_delay(delay.duration(), cx));
        }
        cx.notify();
    }

    fn fade_delay_control(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .text_size(px(13.))
            .child(self.text("Fade delay"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        button("shorter-delay", "−".into(), false)
                            .when(self.fade_delay.step(false) == self.fade_delay, |el| {
                                el.opacity(0.4)
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.adjust_delay(false, cx)
                            })),
                    )
                    .child(
                        div()
                            .min_w(px(46.))
                            .text_center()
                            .child(format!("{:.1} s", self.fade_delay.duration().as_secs_f64())),
                    )
                    .child(
                        button("longer-delay", "+".into(), false)
                            .when(self.fade_delay.step(true) == self.fade_delay, |el| {
                                el.opacity(0.4)
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.adjust_delay(true, cx)
                            })),
                    ),
            )
    }

    fn super_key_control(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .text_size(px(13.))
            .child(self.text("Super key display"))
            .child(
                div()
                    .flex()
                    .gap(px(5.))
                    .children(SuperKeyStyle::ALL.map(|style| {
                        button(
                            style.label(),
                            self.text(style.label()),
                            self.super_key_style == style,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.super_key_style = style;
                                if style.save().is_err() {
                                    this.error = Some(CaptureError::SuperKeyStyle);
                                } else if this.error == Some(CaptureError::SuperKeyStyle) {
                                    this.error = None;
                                }
                                if let Some(overlay) = this.overlay {
                                    let _ = overlay.update(cx, |view, _, cx| {
                                        view.set_super_key_style(style, cx)
                                    });
                                }
                                cx.notify();
                            },
                        ))
                    })),
            )
    }

    fn position_picker(&self, cx: &mut Context<Self>) -> Div {
        let label = self
            .position_profiles
            .iter()
            .position(|position| *position == self.position)
            .map(|index| self.profile_label(index))
            .unwrap_or_else(|| self.text(self.position.label()));
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
                    .child(button("position", label, false).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_position_picker(cx)),
                    )),
            )
            .when_some(self.pending_position, |element, selected| {
                element
                    .children(Position::ALL.chunks(3).map(|row| {
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
                    .child(self.profile_picker(cx))
            })
    }

    fn profile_label(&self, index: usize) -> String {
        format!("{} {}", self.text("Profile"), index + 1)
    }

    fn edit_overlay_placement(&mut self, cx: &mut Context<Self>) {
        if !self.layer_shell
            || self
                .placement_editor
                .is_some_and(|editor| editor.read(cx).is_ok())
        {
            return;
        }
        let controls = cx.entity().downgrade();
        let labels = [
            self.text("Drag the overlay"),
            self.text("Save profile"),
            self.text("Cancel"),
            self.text("Could not save the profile"),
        ];
        self.clear_overlay(cx);
        match overlay::edit_placement(
            cx,
            self.position,
            self.super_key_style,
            labels,
            move |position, cx| {
                controls
                    .update(cx, |this, cx| this.add_position_profile(position, cx))
                    .unwrap_or(false)
            },
        ) {
            Ok(editor) => self.placement_editor = Some(editor),
            Err(_) => {
                self.error = Some(CaptureError::Overlay);
                cx.notify();
            }
        }
    }

    fn add_position_profile(&mut self, position: Position, cx: &mut Context<Self>) -> bool {
        let mut profiles = self.position_profiles.clone();
        if !profiles.contains(&position) {
            profiles.push(position);
        }
        self.save_position_profiles(profiles, position, cx)
    }

    fn save_position_profiles(
        &mut self,
        profiles: Vec<Position>,
        position: Position,
        cx: &mut Context<Self>,
    ) -> bool {
        if Position::save_profiles(&profiles).is_ok() {
            self.position_profiles = profiles;
            if self.error == Some(CaptureError::Position) {
                self.error = None;
            }
            self.pick_position(position, cx)
        } else {
            self.error = Some(CaptureError::Position);
            cx.notify();
            false
        }
    }

    fn profile_picker(&self, cx: &mut Context<Self>) -> Div {
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
        let map_bounds = bounds.clone();
        let marker = match self.profile_draft.unwrap_or(self.position) {
            Position::Custom { x, y } => Some((x, y)),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(13.))
            .child(
                button("drag-overlay", self.text("Drag overlay"), false).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.edit_overlay_placement(cx)),
                ),
            )
            .child(self.text("Click the map to place the overlay"))
            .child(
                div()
                    .id("position-map")
                    .relative()
                    .h(px(90.))
                    .rounded(px(4.))
                    .bg(rgba(0xffffff17))
                    .cursor_pointer()
                    .child(
                        gpui::canvas(move |bounds, _, _| map_bounds.set(bounds), |_, _, _, _| {})
                            .size_full(),
                    )
                    .when_some(marker, |map, (x, y)| {
                        map.child(
                            div()
                                .absolute()
                                .left(gpui::relative(f32::from(x) / 1000.))
                                .top(gpui::relative(f32::from(y) / 1000.))
                                .ml(px(-4.))
                                .mt(px(-4.))
                                .size(px(8.))
                                .rounded_full()
                                .bg(rgb(TEXT)),
                        )
                    })
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        let bounds: Bounds<gpui::Pixels> = bounds.get();
                        let offset = event.position() - bounds.origin;
                        let coordinate = |offset: gpui::Pixels, length: gpui::Pixels| {
                            (1000. * (offset / length).clamp(0., 1.)).round() as u16
                        };
                        this.profile_draft = Some(Position::Custom {
                            x: coordinate(offset.x, bounds.size.width),
                            y: coordinate(offset.y, bounds.size.height),
                        });
                        cx.notify();
                    })),
            )
            .child(
                div().flex().flex_wrap().gap(px(5.)).children(
                    self.position_profiles
                        .iter()
                        .enumerate()
                        .map(|(index, &position)| {
                            button(
                                ("position-profile", index),
                                self.profile_label(index),
                                self.position == position,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.pick_position(position, cx);
                                },
                            ))
                        }),
                ),
            )
            .when_some(self.profile_draft, |picker, position| {
                picker.child(
                    button("save-profile", self.text("Save profile"), true).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.add_position_profile(position, cx);
                        },
                    )),
                )
            })
            .when_some(
                self.profile_draft
                    .filter(|draft| !self.position_profiles.contains(draft)),
                |picker, position| {
                    let active = self
                        .position_profiles
                        .iter()
                        .position(|entry| *entry == self.position);
                    picker.when_some(active, |picker, index| {
                        picker.child(
                            button("update-profile", self.text("Update active profile"), false)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let mut profiles = this.position_profiles.clone();
                                    profiles[index] = position;
                                    this.save_position_profiles(profiles, position, cx);
                                })),
                        )
                    })
                },
            )
            .when(self.position_profiles.contains(&self.position), |picker| {
                picker.child(
                    button("delete-profile", self.text("Delete active profile"), false).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            let mut profiles = this.position_profiles.clone();
                            profiles.retain(|position| *position != this.position);
                            this.save_position_profiles(profiles, Position::default(), cx);
                        }),
                    ),
                )
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
            "Space  Start or pause capture\nP  Choose overlay position\n[ / ]  Adjust fade delay\nQ  Quit\n?  Show keyboard shortcuts"
        } else if let Some(error) = self.error {
            error.message()
        } else if self.capturing {
            "Capture continues after you close this window."
        } else {
            "Show keypresses over the other apps on your screen."
        };

        #[cfg(target_os = "linux")]
        let chrome = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(keyboard_mark())
                    .child("Saiki"),
            )
            .child(chrome_button("minimize", "−", |_, window, _| {
                window.minimize_window()
            }))
            .child(chrome_button("close", "×", |_, window, _| {
                window.remove_window()
            }));

        let body = div()
            .w_full()
            .max_w(px(440.))
            .flex()
            .flex_col()
            .gap(px(12.))
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
                                el.bg(rgb(0xdadbea)).text_color(rgb(0x1b1b1b)).child("✓")
                            }),
                    )
                    .child(self.text(if cfg!(target_os = "windows") {
                        "Only show Ctrl, Alt or Windows shortcuts"
                    } else {
                        "Only show Ctrl, Alt or Super shortcuts"
                    })),
            )
            .child(self.position_picker(cx))
            .child(self.fade_delay_control(cx))
            .child(self.super_key_control(cx))
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
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle(cx))),
                    )
                    .child(
                        button("preview", self.text("Preview"), false)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.preview(cx))),
                    ),
            )
            .child(self.update_control(cx));

        let window = div()
            .track_focus(&self.focus)
            .key_context("Saiki")
            .on_action(cx.listener(|this, _: &ToggleCapture, _, cx| this.toggle(cx)))
            .on_action(
                cx.listener(|this, _: &ChoosePosition, _, cx| this.toggle_position_picker(cx)),
            )
            .on_action(cx.listener(|this, _: &ShorterDelay, _, cx| this.adjust_delay(false, cx)))
            .on_action(cx.listener(|this, _: &LongerDelay, _, cx| this.adjust_delay(true, cx)))
            .on_key_down(cx.listener(Self::position_key))
            .on_action(cx.listener(|this, _: &ShowHelp, _, cx| {
                this.pending_position = None;
                this.help = !this.help;
                cx.notify();
            }))
            .on_action(|_: &Quit, _, cx| cx.quit())
            .size_full()
            .text_color(rgb(TEXT))
            .flex();

        #[cfg(target_os = "linux")]
        {
            window
                .bg(rgb(SURFACE))
                .flex_col()
                // Keep window buttons at the top when the compositor tiles Saiki.
                .child(div().flex_shrink_0().px(px(17.)).py(px(12.)).child(chrome))
                .child(
                    div()
                        .id("controls-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(17.))
                        .pb(px(17.))
                        .child(body),
                )
        }

        #[cfg(target_os = "windows")]
        {
            window
                .id("windows-controls")
                .bg(rgb(0x1b1b1b))
                .flex_col()
                .items_center()
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .max_w(px(480.))
                        .flex_shrink_0()
                        .p(px(20.))
                        .child(
                            div()
                                .rounded(px(18.))
                                .bg(rgb(SURFACE))
                                .border_1()
                                .border_color(rgba(EDGE))
                                .p(px(17.))
                                .flex()
                                .flex_col()
                                .gap(px(12.))
                                .child(body),
                        ),
                )
        }
    }
}

#[cfg(target_os = "linux")]
fn keyboard_mark() -> Div {
    let row = || {
        div()
            .flex()
            .gap(px(1.4))
            .children((0..3).map(|_| div().size(px(1.55)).rounded(px(0.4)).bg(rgb(0xd8dcf0))))
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(20.))
        .rounded(px(7.))
        .bg(rgb(0x424651))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .w(px(12.))
                .h(px(9.))
                .gap(px(1.1))
                .border_1()
                .border_color(rgb(0xd8dcf0))
                .rounded(px(2.))
                .child(row())
                .child(div().w(px(5.)).h(px(1.)).rounded(px(0.5)).bg(rgb(0xd8dcf0))),
        )
}

pub(crate) fn button(
    id: impl Into<gpui::ElementId>,
    text: String,
    primary: bool,
) -> gpui::Stateful<Div> {
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

#[cfg(target_os = "linux")]
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
