use super::{CaptureError, GamerMode, SUPER_KEY, Stroke};
use anyhow::Result;
use async_io::Async;
use futures_lite::{StreamExt, future};
use input::event::keyboard::{KeyState, KeyboardEvent, KeyboardEventTrait};
use input::event::pointer::{ButtonState, PointerEvent};
use input::{DeviceCapability, Event, Libinput, LibinputInterface};
use serde_json::Value;
use std::env;
use std::fs::OpenOptions;
use std::future::Future;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;
use xkbcommon::xkb::{self, keysyms::*};
use zbus::zvariant::OwnedObjectPath;

const KEY_SPACE: u32 = 57;

struct ReadOnlyDevices;

impl LibinputInterface for ReadOnlyDevices {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .read(true)
            .custom_flags(flags & !libc::O_ACCMODE)
            .open(path)
            .map(Into::into)
            .map_err(|error| -error.raw_os_error().unwrap_or(libc::EIO))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

pub struct Keyboard {
    input: Libinput,
    ready: Async<OwnedFd>,
    labels: KeyLabels,
    gamer_mode: GamerMode,
}

impl Keyboard {
    pub async fn open(gamer_mode: GamerMode) -> Result<Self, CaptureError> {
        let keyboard = hyprland_keyboard().await.unwrap_or_default();
        let labels = KeyLabels::new(
            keyboard["layout"].as_str().unwrap_or_default(),
            keyboard["variant"].as_str().unwrap_or_default(),
            keyboard["options"].as_str().map(str::to_owned),
            keyboard["active_keymap"].as_str().unwrap_or_default(),
        )
        .ok_or(CaptureError::Layout)?;

        let mut input = Libinput::new_with_udev(ReadOnlyDevices);
        let seat = env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".into());
        input
            .udev_assign_seat(&seat)
            .map_err(|_| CaptureError::Device)?;
        input.dispatch().map_err(|_| CaptureError::Device)?;

        let mut has_keyboard = false;
        for event in &mut input {
            if let Event::Device(input::event::DeviceEvent::Added(device)) = event {
                let device = input::event::EventTrait::device(&device);
                has_keyboard |= device.has_capability(DeviceCapability::Keyboard)
                    && device.keyboard_has_key(KEY_SPACE) == Ok(true);
            }
        }
        if !has_keyboard {
            return Err(CaptureError::Keyboard);
        }

        // Poll a duplicate, so libinput keeps sole ownership of its original descriptor.
        let fd = input
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| CaptureError::Device)?;
        let ready = Async::new(fd).map_err(|_| CaptureError::Device)?;
        Ok(Self {
            input,
            ready,
            labels,
            gamer_mode,
        })
    }

    pub async fn next_strokes(&mut self) -> Result<Vec<Stroke>, CaptureError> {
        self.ready
            .readable()
            .await
            .map_err(|_| CaptureError::Device)?;
        self.input.dispatch().map_err(|_| CaptureError::Device)?;
        Ok(self
            .input
            .by_ref()
            .filter_map(|event| {
                let gamer_mode = self.gamer_mode.load(Ordering::Relaxed);
                match event {
                    Event::Keyboard(KeyboardEvent::Key(key)) => {
                        let pressed = key.key_state() == KeyState::Pressed;
                        // Multiple devices may hold the same input.
                        if key.seat_key_count() != u32::from(pressed) {
                            return None;
                        }
                        self.labels.update(key.key(), pressed, gamer_mode)
                    }
                    Event::Pointer(PointerEvent::Button(button)) if gamer_mode => {
                        let pressed = button.button_state() == ButtonState::Pressed;
                        if button.seat_button_count() != u32::from(pressed) {
                            return None;
                        }
                        // Linux BTN_LEFT through BTN_EXTRA.
                        let number = button.button().checked_sub(0x10f)?;
                        Stroke::mouse_button(number, pressed)
                    }
                    _ => None,
                }
            })
            .collect())
    }
}

async fn hyprland_keyboard() -> Option<Value> {
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none()
        || env::var_os("XKB_DEFAULT_LAYOUT").is_some()
    {
        return None;
    }
    let mut command = async_process::Command::new("hyprctl");
    command.args(["-j", "devices"]).kill_on_drop(true);
    let output = future::or(async { command.output().await.ok() }, async {
        async_io::Timer::after(Duration::from_millis(250)).await;
        None
    })
    .await?;
    let devices: Value = serde_json::from_slice(&output.stdout).ok()?;
    devices["keyboards"]
        .as_array()?
        .iter()
        .find(|keyboard| keyboard["main"] == true)
        .cloned()
}

struct KeyLabels(xkb::State);

impl KeyLabels {
    fn new(layout: &str, variant: &str, options: Option<String>, active: &str) -> Option<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            layout,
            variant,
            options,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        let mut state = xkb::State::new(&keymap);
        for index in 0..keymap.num_layouts() {
            if keymap.layout_get_name(index) == active {
                state.update_mask(0, 0, 0, 0, 0, index);
                break;
            }
        }
        Some(Self(state))
    }

    #[allow(
        non_upper_case_globals,
        reason = "XKB keysyms use their upstream names"
    )]
    fn update(&mut self, code: u32, pressed: bool, gamer_mode: bool) -> Option<Stroke> {
        // XKB numbers evdev keys with an offset of eight.
        let key = xkb::Keycode::new(code + 8);
        let symbol = if gamer_mode {
            let layout = self.0.key_get_layout(key);
            self.0
                .get_keymap()
                .key_get_syms_by_level(key, layout, 0)
                .first()
                .copied()
                .unwrap_or(xkb::Keysym::NoSymbol)
        } else {
            self.0.key_get_one_sym(key)
        };
        let direction = if pressed {
            xkb::KeyDirection::Down
        } else {
            xkb::KeyDirection::Up
        };
        let changed = self.0.update_key(key, direction);
        let mods_changed = changed & (xkb::STATE_MODS_DEPRESSED | xkb::STATE_MODS_LATCHED) != 0;
        if !pressed {
            return gamer_mode.then_some(Stroke::GameRelease { code });
        }
        if symbol == xkb::Keysym::NoSymbol {
            if gamer_mode {
                return Some(Stroke::GamePress {
                    code,
                    label: format!("Key {code}"),
                });
            }
            return None;
        }
        let is_active = |name: &str| self.0.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
        let ctrl_alt = [(xkb::MOD_NAME_CTRL, "Ctrl"), (xkb::MOD_NAME_ALT, "Alt")];
        if !gamer_mode && matches!(symbol.raw(), KEY_Super_L | KEY_Super_R) {
            return Some(Stroke::Key {
                caps: ctrl_alt
                    .into_iter()
                    .filter(|(name, _)| is_active(name))
                    .map(|(_, label)| label.into())
                    .chain(std::iter::once(SUPER_KEY.into()))
                    .collect(),
                shortcut: true,
            });
        }
        if !gamer_mode && mods_changed && !matches!(symbol.raw(), KEY_Caps_Lock | KEY_Num_Lock) {
            return None;
        }
        if !gamer_mode
            && matches!(
                symbol.raw(),
                KEY_Control_L
                    | KEY_Control_R
                    | KEY_Alt_L
                    | KEY_Alt_R
                    | KEY_Meta_L
                    | KEY_Meta_R
                    | KEY_Hyper_L
                    | KEY_Hyper_R
                    | KEY_Shift_L
                    | KEY_Shift_R
                    | KEY_ISO_Level3_Shift
                    | KEY_ISO_Level5_Shift
            )
        {
            return None;
        }

        let shortcut = [xkb::MOD_NAME_CTRL, xkb::MOD_NAME_ALT, xkb::MOD_NAME_LOGO]
            .into_iter()
            .any(is_active);
        if !gamer_mode && !shortcut {
            match symbol.raw() {
                KEY_space => return Some(Stroke::Space),
                KEY_BackSpace => return Some(Stroke::Backspace),
                _ => {}
            }
            if let Some(character) = symbol
                .key_char()
                .filter(|character| !character.is_control())
            {
                return Some(Stroke::Text(character));
            }
        }
        let named = match symbol.raw() {
            KEY_Control_L | KEY_Control_R => "Ctrl",
            KEY_Alt_L | KEY_Alt_R => "Alt",
            KEY_Shift_L | KEY_Shift_R => "Shift",
            KEY_Super_L | KEY_Super_R => SUPER_KEY,
            KEY_ISO_Level3_Shift => "AltGr",
            KEY_BackSpace => "Backspace",
            KEY_Return => "Enter",
            KEY_KP_Enter => "Num Enter",
            KEY_Escape => "Esc",
            KEY_space => "Space",
            KEY_Tab | KEY_ISO_Left_Tab => "Tab",
            KEY_Delete => "Delete",
            KEY_Insert => "Insert",
            KEY_Page_Up => "Page Up",
            KEY_Page_Down => "Page Down",
            KEY_Home => "Home",
            KEY_End => "End",
            KEY_Left => "←",
            KEY_Right => "→",
            KEY_Up => "↑",
            KEY_Down => "↓",
            KEY_Caps_Lock => "Caps Lock",
            KEY_Num_Lock => "Num Lock",
            KEY_Print => "Print",
            KEY_Multi_key => "Compose",
            KEY_XF86AudioMute => "Mute",
            KEY_XF86AudioLowerVolume => "Volume −",
            KEY_XF86AudioRaiseVolume => "Volume +",
            KEY_XF86AudioPlay => "Play/Pause",
            KEY_XF86MonBrightnessUp => "Brightness +",
            KEY_XF86MonBrightnessDown => "Brightness −",
            _ => "",
        };
        let label = if !named.is_empty() {
            named.into()
        } else if let Some(ch) = symbol.key_char().filter(|ch| !ch.is_control()) {
            if (shortcut || gamer_mode) && !is_active("LevelThree") {
                ch.to_uppercase().collect()
            } else {
                ch.to_string()
            }
        } else {
            xkb::keysym_get_name(symbol)
                .replace('_', " ")
                .chars()
                .take(24)
                .collect()
        };
        if gamer_mode {
            return Some(Stroke::GamePress {
                code,
                label: if label.is_empty() {
                    format!("Key {code}")
                } else {
                    label
                },
            });
        }
        if label.is_empty() {
            return None;
        }
        let mut caps: Vec<String> = ctrl_alt
            .into_iter()
            .chain([
                (xkb::MOD_NAME_LOGO, SUPER_KEY),
                ("LevelThree", "AltGr"),
                (xkb::MOD_NAME_SHIFT, "Shift"),
            ])
            .filter(|(name, _)| is_active(name))
            .map(|(_, label)| label.into())
            .collect();

        caps.push(label);
        Some(Stroke::Key { caps, shortcut })
    }
}

// Subscribe before opening any devices so a lock racing with Start can cancel it.
pub async fn while_unlocked<F>(capture: F) -> Result<(), CaptureError>
where
    F: Future<Output = Result<(), CaptureError>>,
{
    let Ok(id) = env::var("XDG_SESSION_ID") else {
        return capture.await;
    };
    let setup = async {
        let connection = zbus::Connection::system().await?;
        let manager = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await?;
        let path: OwnedObjectPath = manager.call("GetSession", &(id.as_str(),)).await?;
        Ok::<_, anyhow::Error>((connection, path))
    };
    let (connection, path) = setup.await.map_err(|_| CaptureError::Session)?;
    let session = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        path,
        "org.freedesktop.login1.Session",
    )
    .await
    .map_err(|_| CaptureError::Session)?;

    let mut lock = session
        .receive_signal("Lock")
        .await
        .map_err(|_| CaptureError::Session)?;
    let inactive = wait_for_property(&session, "Active", false);
    let locked = wait_for_property(&session, "LockedHint", true);
    // Reading the initial values prevents a capture stream in a locked session.
    if !session
        .get_property::<bool>("Active")
        .await
        .map_err(|_| CaptureError::Session)?
        || session
            .get_property::<bool>("LockedHint")
            .await
            .map_err(|_| CaptureError::Session)?
    {
        return Err(CaptureError::Session);
    }
    let lock_signal = async {
        lock.next().await;
    };
    let session_ended = async {
        future::or(inactive, future::or(locked, lock_signal)).await;
        Ok(())
    };
    future::or(session_ended, capture).await
}

async fn wait_for_property(session: &zbus::Proxy<'_>, name: &'static str, value: bool) {
    let mut stream = session.receive_property_changed::<bool>(name).await;
    while let Some(change) = stream.next().await {
        if change.get().await.map_or(true, |current| current == value) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamer_mode_displays_keys_individually_while_modifiers_are_held() {
        let mut labels = KeyLabels::new("us", "", Some(String::new()), "").unwrap();
        for (code, label) in [
            (29, "Ctrl"),
            (56, "Alt"),
            (42, "Shift"),
            (57, "Space"),
            (30, "A"),
            (14, "Backspace"),
        ] {
            assert_eq!(
                labels.update(code, true, true),
                Some(Stroke::GamePress {
                    code,
                    label: label.into()
                })
            );
        }
        assert_eq!(
            labels.update(57, false, true),
            Some(Stroke::GameRelease { code: 57 })
        );
        assert_eq!(
            labels.update(57, true, true),
            Some(Stroke::GamePress {
                code: 57,
                label: "Space".into()
            })
        );
    }

    #[test]
    fn distinguishes_typing_and_shortcuts() {
        let mut labels = KeyLabels::new("us", "", Some(String::new()), "").unwrap();
        assert_eq!(labels.update(30, true, false), Some(Stroke::Text('a')));
        assert!(labels.update(30, false, false).is_none());
        assert!(labels.update(29, true, false).is_none());
        assert!(labels.update(42, true, false).is_none());
        assert_eq!(
            labels.update(46, true, false),
            Some(Stroke::Key {
                caps: vec!["Ctrl".into(), "Shift".into(), "C".into()],
                shortcut: true,
            })
        );
    }

    #[test]
    fn shows_super_on_press_and_in_shortcuts() {
        for code in [125, 126] {
            let mut labels = KeyLabels::new("us", "", Some(String::new()), "").unwrap();
            assert_eq!(
                labels.update(code, true, false),
                Some(Stroke::Key {
                    caps: vec![SUPER_KEY.into()],
                    shortcut: true
                })
            );
            assert_eq!(
                labels.update(KEY_SPACE, true, false),
                Some(Stroke::Key {
                    caps: vec![SUPER_KEY.into(), "Space".into()],
                    shortcut: true,
                })
            );
            assert!(labels.update(code, false, false).is_none());
            labels.update(KEY_SPACE, false, false);
            assert_eq!(labels.update(KEY_SPACE, true, false), Some(Stroke::Space));
        }
    }

    #[test]
    fn includes_held_ctrl_and_alt_when_super_is_pressed() {
        for (modifiers, caps) in [
            (&[29][..], vec!["Ctrl", SUPER_KEY]),
            (&[56][..], vec!["Alt", SUPER_KEY]),
            (&[29, 56][..], vec!["Ctrl", "Alt", SUPER_KEY]),
        ] {
            let mut labels = KeyLabels::new("us", "", Some(String::new()), "").unwrap();
            for &code in modifiers {
                assert!(labels.update(code, true, false).is_none());
            }
            assert_eq!(
                labels.update(125, true, false),
                Some(Stroke::Key {
                    caps: caps.into_iter().map(str::to_owned).collect(),
                    shortcut: true,
                })
            );
        }
    }

    #[test]
    fn maps_active_layout_and_alt_gr() {
        let mut labels = KeyLabels::new("us,de", "", Some(String::new()), "German").unwrap();
        assert_eq!(labels.update(21, true, false), Some(Stroke::Text('z')));
        labels.update(21, false, false);
        assert!(labels.update(100, true, false).is_none());
        assert_eq!(labels.update(16, true, false), Some(Stroke::Text('@')));
    }

    #[test]
    fn opens_desktop_keyboard_when_requested() {
        if env::var_os("SAIKI_TEST_DEVICES").is_some() {
            future::block_on(async {
                let opened =
                    while_unlocked(async { Keyboard::open(GamerMode::default()).await.map(drop) })
                        .await;
                assert_eq!(opened, Ok(()));
            });
        }
    }
}
