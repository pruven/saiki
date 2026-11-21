use gpui::layer_shell::Anchor;
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

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
        let saved = path()
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|position| position.id() == saved.trim())
            .unwrap_or_default()
    }

    pub fn save(self) -> io::Result<()> {
        let Some(path) = path() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no user config directory",
            ));
        };
        let directory = path.parent().expect("position file has a parent directory");
        fs::create_dir_all(directory)?;
        let pending = directory.join(format!(".position-{}", std::process::id()));
        let result = fs::write(&pending, format!("{}\n", self.id()))
            .and_then(|()| fs::rename(&pending, path));
        if result.is_err() {
            let _ = fs::remove_file(pending);
        }
        result
    }
}

fn path() -> Option<PathBuf> {
    let dir = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(dir.join("saiki/position"))
}
