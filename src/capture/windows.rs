use super::{CaptureError, GamerMode, SUPER_KEY, Stroke};
use async_channel::{Receiver, Sender};
use std::{
    cell::RefCell,
    future::Future,
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
};
use windows::Win32::{
    Foundation::*,
    System::{
        LibraryLoader::GetModuleHandleW, RemoteDesktop::*, StationsAndDesktops::*,
        Threading::GetCurrentThreadId,
    },
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

thread_local! {
    // A low-level hook runs on the installing thread. Never wait in its callback.
    static CAPTURE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

struct HookState {
    sender: Sender<Result<Vec<Stroke>, CaptureError>>,
    keys: [u8; 256],
    gamer_mode: GamerMode,
}

pub struct Keyboard {
    receiver: Receiver<Result<Vec<Stroke>, CaptureError>>,
    worker: Arc<Mutex<Worker>>,
}

#[derive(Default)]
struct Worker {
    thread_id: Option<u32>,
    stopped: bool,
}

// Clear the thread ID on every exit path while keeping shutdown synchronized with Drop.
struct WorkerQueue(Arc<Mutex<Worker>>);

impl Drop for WorkerQueue {
    fn drop(&mut self) {
        self.0.lock().unwrap().thread_id = None;
    }
}

impl Keyboard {
    pub async fn open(gamer_mode: GamerMode) -> Result<Self, CaptureError> {
        let (sender, receiver) = async_channel::bounded(64);
        let (ready, started) = async_channel::bounded(1);
        let worker = Arc::new(Mutex::new(Worker::default()));
        let registration = WorkerQueue(worker.clone());
        thread::spawn(move || unsafe {
            // Create the message queue before publishing the thread ID.
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            let _registration = registration;
            {
                let mut worker = _registration.0.lock().unwrap();
                if worker.stopped {
                    return;
                }
                worker.thread_id = Some(GetCurrentThreadId());
            }
            if !session_active() {
                let _ = ready.try_send(Err(CaptureError::Session));
                return;
            }
            let mut keys = [0; 256];
            for (key, value) in keys.iter_mut().enumerate() {
                *value = if GetAsyncKeyState(key as i32) < 0 {
                    0x80
                } else {
                    0
                };
            }
            for key in [VK_CAPITAL, VK_NUMLOCK, VK_SCROLL] {
                keys[key.0 as usize] |= (GetKeyState(key.0 as i32) & 1) as u8;
            }
            CAPTURE.with(|state| {
                *state.borrow_mut() = Some(HookState {
                    sender,
                    keys,
                    gamer_mode,
                })
            });
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook),
                Some(GetModuleHandleW(None).unwrap_or_default().into()),
                0,
            );
            let Ok(hook) = hook else {
                let _ = ready.try_send(Err(CaptureError::Keyboard));
                CAPTURE.with(|state| state.borrow_mut().take());
                return;
            };
            let mouse = SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse_hook),
                Some(GetModuleHandleW(None).unwrap_or_default().into()),
                0,
            );
            let Ok(mouse) = mouse else {
                let _ = UnhookWindowsHookEx(hook);
                CAPTURE.with(|state| state.borrow_mut().take());
                let _ = ready.try_send(Err(CaptureError::Device));
                return;
            };
            // A timer also pauses capture when the session locks without a key event.
            let timer = SetTimer(None, 1, 100, None);
            if timer == 0 {
                let _ = UnhookWindowsHookEx(mouse);
                let _ = UnhookWindowsHookEx(hook);
                CAPTURE.with(|state| state.borrow_mut().take());
                let _ = ready.try_send(Err(CaptureError::Session));
                return;
            }
            let _ = ready.try_send(Ok(()));
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                // Timer messages also wake us if posting WM_QUIT during Drop failed.
                if _registration.0.lock().unwrap().stopped {
                    break;
                }
                if message.message == WM_TIMER && !session_active() {
                    CAPTURE.with(|state| {
                        if let Some(state) = state.borrow().as_ref() {
                            let _ = state.sender.force_send(Err(CaptureError::Session));
                        }
                    });
                    break;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let _ = KillTimer(None, timer);
            let _ = UnhookWindowsHookEx(mouse);
            let _ = UnhookWindowsHookEx(hook);
            CAPTURE.with(|state| state.borrow_mut().take());
        });
        // Own the worker before awaiting startup so cancelling Start also stops the hook.
        let keyboard = Self { receiver, worker };
        started.recv().await.map_err(|_| CaptureError::Device)??;
        Ok(keyboard)
    }

    pub async fn next_strokes(&mut self) -> Result<Vec<Stroke>, CaptureError> {
        self.receiver
            .recv()
            .await
            .map_err(|_| CaptureError::Device)?
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        let mut worker = self.worker.lock().unwrap();
        worker.stopped = true;
        if let Some(thread_id) = worker.thread_id {
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        // The worker owns its hook and cleans it up on exit. Never join on the UI executor.
    }
}

pub async fn while_unlocked<F: Future<Output = Result<(), CaptureError>>>(
    capture: F,
) -> Result<(), CaptureError> {
    capture.await
}

// The input desktop is inaccessible on the lock screen and secure UAC desktop.
fn session_active() -> bool {
    unsafe {
        let mut data = windows::core::PWSTR::null();
        let mut bytes = 0;
        if WTSQuerySessionInformationW(
            None,
            WTS_CURRENT_SESSION,
            WTSConnectState,
            &mut data,
            &mut bytes,
        )
        .is_err()
        {
            return false;
        }
        let active = bytes as usize >= std::mem::size_of::<WTS_CONNECTSTATE_CLASS>()
            && *(data.0 as *const WTS_CONNECTSTATE_CLASS) == WTSActive;
        WTSFreeMemory(data.0.cast());
        if !active {
            return false;
        }
        let Ok(desktop) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        else {
            return false;
        };
        let mut name = [0u16; 64];
        let mut needed = 0;
        let readable = GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            Some(&mut needed),
        )
        .is_ok();
        let _ = CloseDesktop(desktop);
        readable
            && String::from_utf16_lossy(
                &name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())],
            )
            .eq_ignore_ascii_case("Default")
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let pressed = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        queue_strokes(|state| state.update(event, pressed));
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        queue_strokes(|state| {
            if !state.gamer_mode.load(Ordering::Relaxed) {
                return None;
            }
            let message = wparam.0 as u32;
            let (button, pressed) = match message {
                WM_LBUTTONDOWN | WM_LBUTTONUP => (1, message == WM_LBUTTONDOWN),
                WM_RBUTTONDOWN | WM_RBUTTONUP => (2, message == WM_RBUTTONDOWN),
                WM_MBUTTONDOWN | WM_MBUTTONUP => (3, message == WM_MBUTTONDOWN),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
                    (3 + (event.mouseData >> 16), message == WM_XBUTTONDOWN)
                }
                _ => return None,
            };
            Stroke::mouse_button(button, pressed).map(|stroke| vec![stroke])
        });
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn queue_strokes(strokes: impl FnOnce(&mut HookState) -> Option<Vec<Stroke>>) {
    CAPTURE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut()
            && let Some(strokes) = strokes(state)
            && state.sender.try_send(Ok(strokes)).is_err()
        {
            // Overflow stops capture rather than retaining an unbounded backlog.
            let _ = state.sender.force_send(Err(CaptureError::Device));
            unsafe { PostQuitMessage(0) };
        }
    });
}

impl HookState {
    fn update(&mut self, event: &KBDLLHOOKSTRUCT, pressed: bool) -> Option<Vec<Stroke>> {
        let key = event.vkCode as usize;
        if key >= self.keys.len() {
            return None;
        }
        let was_down = self.keys[key] & 0x80 != 0;
        self.keys[key] = (self.keys[key] & 1) | if pressed { 0x80 } else { 0 };
        if pressed && !was_down && matches!(key, 0x14 | 0x90 | 0x91) {
            self.keys[key] ^= 1;
        }
        for (generic, left, right) in [(0x10, 0xa0, 0xa1), (0x11, 0xa2, 0xa3), (0x12, 0xa4, 0xa5)] {
            self.keys[generic] = self.keys[left] | self.keys[right];
        }
        let gamer_mode = self.gamer_mode.load(Ordering::Relaxed);
        if !pressed {
            return (gamer_mode && was_down)
                .then(|| vec![Stroke::GameRelease { code: event.vkCode }]);
        }
        if gamer_mode && was_down {
            return None;
        }
        if !gamer_mode && matches!(key, 0x10..=0x12 | 0xa0..=0xa5) {
            return None;
        }
        let down = |key| self.keys[key] & 0x80 != 0;
        let alt_gr = down(0xa5) && down(0x11);
        let ctrl = down(0x11) && !alt_gr;
        let alt = down(0x12) && !alt_gr;
        let ctrl_alt = [(ctrl, "Ctrl"), (alt, "Alt")];
        if !gamer_mode && matches!(VIRTUAL_KEY(key as u16), VK_LWIN | VK_RWIN) {
            return (!was_down).then(|| {
                vec![Stroke::Key {
                    caps: ctrl_alt
                        .into_iter()
                        .filter(|(active, _)| *active)
                        .map(|(_, label)| label.into())
                        .chain(std::iter::once(SUPER_KEY.into()))
                        .collect(),
                    shortcut: true,
                }]
            });
        }
        let win = down(VK_LWIN.0 as usize) || down(VK_RWIN.0 as usize);
        let shift = down(0x10);
        let shortcut = ctrl || alt || win;
        if !gamer_mode && !shortcut {
            if key == 0x20 {
                return Some(vec![Stroke::Space]);
            }
            if key == 0x08 {
                return Some(vec![Stroke::Backspace]);
            }
        }
        let mut keys = self.keys;
        if gamer_mode {
            keys = [0; 256];
        } else if shortcut {
            // Ctrl would otherwise turn letters into control characters.
            for key in [0x11, 0xa2, 0xa3, 0x12, 0xa4, 0xa5] {
                keys[key] = 0;
            }
        }
        let mut text = [0u16; 16];
        let count = unsafe {
            let foreground = GetForegroundWindow();
            let thread = GetWindowThreadProcessId(foreground, None);
            ToUnicodeEx(
                event.vkCode,
                event.scanCode,
                &keys,
                &mut text,
                4,
                Some(GetKeyboardLayout(thread)),
            )
        };
        let translated = if count > 0 {
            String::from_utf16_lossy(&text[..(count as usize).min(text.len())])
        } else {
            String::new()
        };
        if gamer_mode {
            let label = key_label(key as u32, &translated).unwrap_or_else(|| format!("Key {key}"));
            return Some(vec![Stroke::GamePress {
                code: event.vkCode,
                label: if label.chars().count() == 1 {
                    label.to_uppercase()
                } else {
                    label
                },
            }]);
        }
        if !shortcut && !translated.is_empty() && translated.chars().all(|c| !c.is_control()) {
            return Some(translated.chars().map(Stroke::Text).collect());
        }
        let label = key_label(key as u32, &translated)?;
        let caps = ctrl_alt
            .into_iter()
            .chain([(win, SUPER_KEY), (alt_gr, "AltGr"), (shift, "Shift")])
            .filter(|(active, _)| *active)
            .map(|(_, label)| label.to_owned())
            .chain(Some(if shortcut {
                if label.chars().count() == 1 {
                    label.to_uppercase()
                } else {
                    label
                }
            } else {
                label
            }))
            .collect();
        Some(vec![Stroke::Key { caps, shortcut }])
    }
}

fn key_label(key: u32, translated: &str) -> Option<String> {
    let named = match key {
        0x10 | 0xa0 | 0xa1 => "Shift",
        0x11 | 0xa2 | 0xa3 => "Ctrl",
        0x12 | 0xa4 | 0xa5 => "Alt",
        0x5b | 0x5c => SUPER_KEY,
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0d => "Enter",
        0x1b => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "←",
        0x26 => "↑",
        0x27 => "→",
        0x28 => "↓",
        0x2c => "Print",
        0x2d => "Insert",
        0x2e => "Delete",
        0x14 => "Caps Lock",
        0x90 => "Num Lock",
        0x91 => "Scroll Lock",
        0x13 => "Pause",
        0xad => "Mute",
        0xae => "Volume −",
        0xaf => "Volume +",
        0xb3 => "Play/Pause",
        _ => "",
    };
    if !named.is_empty() {
        Some(named.into())
    } else if (0x70..=0x87).contains(&key) {
        Some(format!("F{}", key - 0x6f))
    } else if !translated.is_empty() && translated.chars().all(|c| !c.is_control()) {
        Some(translated.into())
    } else if (0x30..=0x39).contains(&key) || (0x41..=0x5a).contains(&key) {
        char::from_u32(key).map(String::from)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(state: &mut HookState, key: u32, pressed: bool) -> Option<Vec<Stroke>> {
        state.update(
            &KBDLLHOOKSTRUCT {
                vkCode: key,
                ..Default::default()
            },
            pressed,
        )
    }

    #[test]
    fn gamer_mode_displays_keys_individually_and_ignores_auto_repeat() {
        let (sender, _) = async_channel::bounded(1);
        let mut state = HookState {
            sender,
            keys: [0; 256],
            gamer_mode: Arc::new(true.into()),
        };
        for (key, label) in [
            (0xa2, "Ctrl"),
            (0xa4, "Alt"),
            (0xa0, "Shift"),
            (0x20, "Space"),
            (0x08, "Backspace"),
        ] {
            assert_eq!(
                update(&mut state, key, true),
                Some(vec![Stroke::GamePress {
                    code: key,
                    label: label.into()
                }])
            );
        }
        assert!(update(&mut state, 0x20, true).is_none());
        assert_eq!(
            update(&mut state, 0x20, false),
            Some(vec![Stroke::GameRelease { code: 0x20 }])
        );
        assert_eq!(
            update(&mut state, 0x20, true),
            Some(vec![Stroke::GamePress {
                code: 0x20,
                label: "Space".into()
            }])
        );
    }

    #[test]
    fn shows_either_windows_key_on_press_and_in_shortcuts() {
        for key in [VK_LWIN, VK_RWIN] {
            let (sender, _) = async_channel::bounded(1);
            let mut state = HookState {
                sender,
                keys: [0; 256],
                gamer_mode: GamerMode::default(),
            };
            let key = u32::from(key.0);
            let standalone = Some(vec![Stroke::Key {
                caps: vec![SUPER_KEY.into()],
                shortcut: true,
            }]);
            assert_eq!(update(&mut state, key, true), standalone);
            assert!(update(&mut state, key, true).is_none());
            assert_eq!(
                update(&mut state, u32::from(VK_SPACE.0), true),
                Some(vec![Stroke::Key {
                    caps: vec![SUPER_KEY.into(), "Space".into()],
                    shortcut: true,
                }])
            );
            assert!(update(&mut state, key, false).is_none());
            assert_eq!(
                update(&mut state, u32::from(VK_SPACE.0), true),
                Some(vec![Stroke::Space])
            );
            assert_eq!(update(&mut state, key, true), standalone);
        }
    }

    #[test]
    fn includes_held_ctrl_and_alt_when_windows_key_is_pressed() {
        for (modifiers, caps) in [
            (&[VK_LCONTROL][..], vec!["Ctrl", SUPER_KEY]),
            (&[VK_LMENU][..], vec!["Alt", SUPER_KEY]),
            (&[VK_LCONTROL, VK_LMENU][..], vec!["Ctrl", "Alt", SUPER_KEY]),
            (&[VK_LCONTROL, VK_RMENU][..], vec![SUPER_KEY]),
        ] {
            let (sender, _) = async_channel::bounded(1);
            let mut state = HookState {
                sender,
                keys: [0; 256],
                gamer_mode: GamerMode::default(),
            };
            for key in modifiers {
                assert!(update(&mut state, u32::from(key.0), true).is_none());
            }
            assert_eq!(
                update(&mut state, u32::from(VK_LWIN.0), true),
                Some(vec![Stroke::Key {
                    caps: caps.into_iter().map(str::to_owned).collect(),
                    shortcut: true,
                }])
            );
        }
    }

    #[test]
    fn tracks_modifiers_and_keeps_named_keys() {
        let (sender, _) = async_channel::bounded(1);
        let mut state = HookState {
            sender,
            keys: [0; 256],
            gamer_mode: GamerMode::default(),
        };
        assert!(update(&mut state, 0xa2, true).is_none());
        assert!(update(&mut state, 0xa0, true).is_none());
        assert_eq!(
            update(&mut state, 0x0d, true),
            Some(vec![Stroke::Key {
                caps: vec!["Ctrl".into(), "Shift".into(), "Enter".into()],
                shortcut: true,
            }])
        );
        update(&mut state, 0xa2, false);
        update(&mut state, 0xa0, false);
        assert_eq!(update(&mut state, 0x20, true), Some(vec![Stroke::Space]));
        assert_eq!(
            update(&mut state, 0x08, true),
            Some(vec![Stroke::Backspace])
        );
    }

    #[test]
    fn labels_function_and_navigation_keys() {
        assert_eq!(key_label(0x7b, ""), Some("F12".into()));
        assert_eq!(key_label(0x25, ""), Some("←".into()));
        assert_eq!(key_label(0xff, ""), None);
    }
}
