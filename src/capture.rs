use std::sync::{Arc, atomic::AtomicBool};

pub type GamerMode = Arc<AtomicBool>;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
pub use linux::{Keyboard, while_unlocked};
#[cfg(target_os = "windows")]
pub use windows::{Keyboard, while_unlocked};

pub const SUPER_KEY: &str = "Super";
pub const MOUSE_BUTTON_BASE: u32 = 0x8000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    Keyboard,
    #[cfg(target_os = "linux")]
    Layout,
    Session,
    Device,
    Overlay,
    Position,
    FadeDelay,
    Appearance,
    SuperKeyStyle,
}

impl CaptureError {
    pub fn message(self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            Self::Keyboard => "No readable keyboard. Check access to /dev/input/event*.",
            #[cfg(target_os = "windows")]
            Self::Keyboard => "Could not start Windows keyboard capture.",
            #[cfg(target_os = "linux")]
            Self::Layout => "The keyboard layout could not be loaded.",
            Self::Session => "Start capture from an active, unlocked desktop session.",
            Self::Device => "Keyboard capture stopped. Try starting it again.",
            #[cfg(target_os = "windows")]
            Self::Overlay => "Could not open the overlay window.",
            #[cfg(target_os = "linux")]
            Self::Overlay => "This Wayland compositor needs layer-shell support.",
            Self::Position => "Could not save the overlay position.",
            Self::Appearance => "Could not save the appearance settings.",
            Self::FadeDelay => "Could not save the fade delay.",
            Self::SuperKeyStyle => "Could not save the Super key display setting.",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Stroke {
    Text(char),
    Space,
    Backspace,
    GamePress { code: u32, label: String },
    GameRelease { code: u32 },
    Key { caps: Vec<String>, shortcut: bool },
}

impl Stroke {
    pub fn mouse_button(button: u32, pressed: bool) -> Option<Self> {
        let label = match button {
            1 => "LMB",
            2 => "RMB",
            3 => "MMB",
            4 => "Mouse 4",
            5 => "Mouse 5",
            _ => return None,
        };
        let code = MOUSE_BUTTON_BASE + button;
        Some(if pressed {
            Self::GamePress {
                code,
                label: label.into(),
            }
        } else {
            Self::GameRelease { code }
        })
    }

    pub fn is_shortcut(&self) -> bool {
        matches!(self, Self::Key { shortcut: true, .. })
    }
}
