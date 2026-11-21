#[cfg(target_os = "linux")]
use gpui::layer_shell::Anchor;
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopCenter,
    #[default]
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Position {
    pub const ALL: [Self; 6] = [
        Self::TopLeft,
        Self::TopCenter,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomCenter,
        Self::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopCenter => "Top center",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomCenter => "Bottom center",
            Self::BottomRight => "Bottom right",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomCenter => "bottom-center",
            Self::BottomRight => "bottom-right",
        }
    }

    pub fn is_bottom(self) -> bool {
        matches!(
            self,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight
        )
    }

    pub fn horizontal(self) -> i8 {
        match self {
            Self::TopLeft | Self::BottomLeft => -1,
            Self::TopCenter | Self::BottomCenter => 0,
            Self::TopRight | Self::BottomRight => 1,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn anchor(self) -> Anchor {
        let vertical = if self.is_bottom() {
            Anchor::BOTTOM
        } else {
            Anchor::TOP
        };
        let horizontal = match self.horizontal() {
            -1 => Anchor::LEFT,
            1 => Anchor::RIGHT,
            _ => Anchor::empty(),
        };
        vertical | horizontal
    }

    pub fn load() -> Self {
        let saved = path("position")
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|position| position.id() == saved.trim())
            .unwrap_or_default()
    }

    pub fn save(self) -> io::Result<()> {
        save("position", self.id())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FadeDelay(u64);

impl Default for FadeDelay {
    fn default() -> Self {
        Self(2600)
    }
}

impl FadeDelay {
    const MIN: u64 = 500;
    const MAX: u64 = 10_000;

    pub fn load() -> Self {
        path("fade-delay-ms")
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|value| value.trim().parse().ok())
            .filter(|value| (Self::MIN..=Self::MAX).contains(value))
            .map(Self)
            .unwrap_or_default()
    }

    pub fn save(self) -> io::Result<()> {
        save("fade-delay-ms", &self.0.to_string())
    }

    pub fn step(self, longer: bool) -> Self {
        let delta = if longer { 500 } else { -500 };
        Self(
            self.0
                .saturating_add_signed(delta)
                .clamp(Self::MIN, Self::MAX),
        )
    }

    pub fn duration(self) -> Duration {
        Duration::from_millis(self.0)
    }
}

fn path(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let dir = env::var_os("APPDATA").map(PathBuf::from)?;
    #[cfg(target_os = "linux")]
    let dir = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(dir.join("saiki").join(name))
}

fn save(name: &str, value: &str) -> io::Result<()> {
    let path = path(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;
    let directory = path.parent().expect("settings file has a parent directory");
    fs::create_dir_all(directory)?;
    let pending = directory.join(format!(".{name}-{}", std::process::id()));
    let result = fs::write(&pending, format!("{value}\n")).and_then(|()| replace(&pending, &path));
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result
}

#[cfg(target_os = "linux")]
fn replace(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(target_os = "windows")]
fn replace(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW},
        core::PCWSTR,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING,
        )
    }
    .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_an_existing_settings_file() {
        let directory = env::temp_dir().join(format!(
            "saiki-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let saved = directory.join("position");
        let pending = directory.join("pending");
        fs::write(&saved, "top-left\n").unwrap();
        fs::write(&pending, "bottom-right\n").unwrap();
        replace(&pending, &saved).unwrap();
        assert_eq!(fs::read_to_string(&saved).unwrap(), "bottom-right\n");
        assert!(!pending.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
