//! What rsdisk remembers between runs (spec §3.12): theme names per appearance and the last
//! visual and colour mode.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visual {
    #[default]
    Treemap,
    Sunburst,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColourMode {
    #[default]
    Kind,
    Age,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub light_theme: Option<String>,
    pub dark_theme: Option<String>,
    pub visual: Visual,
    pub colour: ColourMode,
}

impl Settings {
    /// `~/Library/Application Support/rsdisk/settings.json`.
    pub fn default_path() -> Option<PathBuf> {
        std::env::home_dir().map(|h| h.join("Library/Application Support/rsdisk/settings.json"))
    }

    /// The saved settings, or defaults when the file is missing or unreadable. A broken file
    /// is left alone; only an explicit `save` replaces it.
    pub fn load(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/settings.json");
        let s = Settings {
            light_theme: Some("One Light".into()),
            dark_theme: Some("Tokyo Night".into()),
            visual: Visual::Sunburst,
            colour: ColourMode::Age,
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            Settings::load(&dir.path().join("nope.json")),
            Settings::default()
        );
    }

    #[test]
    fn corrupt_file_gives_defaults_and_is_kept() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, br#"{"dark_theme": "Ayu Dark", "future": 1}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.dark_theme.as_deref(), Some("Ayu Dark"));
        assert_eq!(s.visual, Visual::Treemap);
    }
}
