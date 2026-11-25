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
    Custom {
        x: u16,
        y: u16,
    },
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
            Self::Custom { .. } => "Custom",
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
            Self::Custom { .. } => "custom",
        }
    }

    pub fn is_bottom(self) -> bool {
        matches!(
            self,
            Self::BottomLeft
                | Self::BottomCenter
                | Self::BottomRight
                | Self::Custom { y: 500.., .. }
        )
    }

    pub fn horizontal(self) -> i8 {
        match self {
            Self::TopLeft | Self::BottomLeft => -1,
            Self::TopCenter | Self::BottomCenter | Self::Custom { .. } => 0,
            Self::TopRight | Self::BottomRight => 1,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn anchor(self) -> Anchor {
        if matches!(self, Self::Custom { .. }) {
            return Anchor::TOP | Anchor::LEFT;
        }
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
        Self::parse(saved.trim()).unwrap_or_default()
    }

    fn parse(value: &str) -> Option<Self> {
        if let Some((x, y)) = value.split_once(',') {
            let (x, y) = (x.parse().ok()?, y.parse().ok()?);
            return (x <= 1000 && y <= 1000).then_some(Self::Custom { x, y });
        }
        Self::ALL
            .into_iter()
            .find(|position| position.id() == value)
    }

    fn encode(self) -> String {
        match self {
            Self::Custom { x, y } => format!("{x},{y}"),
            _ => self.id().into(),
        }
    }

    pub fn save(self) -> io::Result<()> {
        save("position", &self.encode())
    }

    pub fn load_profiles() -> Vec<Self> {
        path("position-profiles")
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default()
            .lines()
            .filter_map(Self::parse)
            .filter(|position| matches!(position, Self::Custom { .. }))
            .collect()
    }

    pub fn save_profiles(profiles: &[Self]) -> io::Result<()> {
        let entries: Vec<_> = profiles.iter().map(|position| position.encode()).collect();
        save("position-profiles", &entries.join("\n"))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuperKeyStyle {
    Text,
    WindowsIcon,
}

impl Default for SuperKeyStyle {
    fn default() -> Self {
        if cfg!(target_os = "windows") {
            Self::WindowsIcon
        } else {
            Self::Text
        }
    }
}

impl SuperKeyStyle {
    pub const ALL: [Self; 2] = [Self::Text, Self::WindowsIcon];

    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "Super",
            Self::WindowsIcon => "Windows icon",
        }
    }

    pub fn load() -> Self {
        let saved = path("super-key-style").and_then(|path| fs::read_to_string(path).ok());
        match saved.as_deref().map(str::trim) {
            Some("text") => Self::Text,
            Some("windows-icon") => Self::WindowsIcon,
            _ => Self::default(),
        }
    }

    pub fn save(self) -> io::Result<()> {
        save(
            "super-key-style",
            match self {
                Self::Text => "text",
                Self::WindowsIcon => "windows-icon",
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub font_size: u8,
    pub opacity: u8,
    pub spacing: u8,
    pub max_cards: u8,
    pub palette: u8,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_size: 17,
            opacity: 100,
            spacing: 7,
            max_cards: 4,
            palette: 0,
        }
    }
}

impl Appearance {
    pub const FONT_SIZE: std::ops::RangeInclusive<u8> = 12..=24;
    pub const OPACITY: std::ops::RangeInclusive<u8> = 25..=100;
    pub const SPACING: std::ops::RangeInclusive<u8> = 0..=19;
    pub const MAX_CARDS: u8 = 6;
    pub const PALETTES: [&str; 4] = ["Default", "Light", "Ocean", "Plum"];

    pub fn load() -> Self {
        let saved = path("appearance.json")
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
            .unwrap_or_default();
        let defaults = Self::default();
        let read = |key: &str, range: std::ops::RangeInclusive<u8>, default| {
            saved[key]
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .filter(|value| range.contains(value))
                .unwrap_or(default)
        };
        Self {
            font_size: read("font_size", Self::FONT_SIZE, defaults.font_size),
            opacity: read("opacity", Self::OPACITY, defaults.opacity),
            spacing: read("spacing", Self::SPACING, defaults.spacing),
            max_cards: read("max_cards", 1..=Self::MAX_CARDS, defaults.max_cards),
            palette: read(
                "palette",
                0..=(Self::PALETTES.len() - 1) as u8,
                defaults.palette,
            ),
        }
    }

    pub fn save(self) -> io::Result<()> {
        save(
            "appearance.json",
            &serde_json::json!({
                "font_size": self.font_size,
                "opacity": self.opacity,
                "spacing": self.spacing,
                "max_cards": self.max_cards,
                "palette": self.palette,
            })
            .to_string(),
        )
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
    let result =
        fs::write(&pending, format!("{value}\n")).and_then(|()| fs::rename(&pending, &path));
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_positions_round_trip_and_reject_out_of_range_coordinates() {
        let position = Position::Custom { x: 250, y: 1000 };
        assert_eq!(Position::parse(&position.encode()), Some(position));
        assert_eq!(Position::parse("250,1001"), None);
        assert_eq!(Position::parse("-1,500"), None);
        assert_eq!(Position::parse("custom"), None);
        assert_eq!(Position::parse("top-right"), Some(Position::TopRight));
    }

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
        fs::rename(&pending, &saved).unwrap();
        assert_eq!(fs::read_to_string(&saved).unwrap(), "bottom-right\n");
        assert!(!pending.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
