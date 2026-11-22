#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
pub use linux::{Keyboard, while_unlocked};
#[cfg(target_os = "windows")]
pub use windows::{Keyboard, while_unlocked};

pub const SUPER_KEY: &str = "Super";

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
    Key { caps: Vec<String>, shortcut: bool },
}

impl Stroke {
    pub fn is_shortcut(&self) -> bool {
        matches!(self, Self::Key { shortcut: true, .. })
    }
}
