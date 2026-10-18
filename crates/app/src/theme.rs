//! Colour themes (spec §3.12): two built-ins plus every theme in the Zed theme files installed
//! on this Mac, mapped onto rsdisk's colour roles. The loading is copied from riff
//! (`~/repos/private/riff/src/theme.rs`), minus its loop and section roles.

use gpui_kit::component::{Theme as UiTheme, ThemeMode};
use gpui_kit::{App, Hsla, Window, rgb};
use rsdisk_core::kind::Kind;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub bg: u32,
    pub surface: u32,
    /// The selected row.
    pub raised: u32,
    pub text: u32,
    pub dim: u32,
    /// Borders and separators.
    pub faint: u32,
    pub accent: u32,
    pub error: u32,
    /// One colour per file kind, in the order of `Kind::ALL`.
    pub kinds: [u32; 8],
}

impl Theme {
    pub fn kind(&self, kind: Kind) -> u32 {
        self.kinds[kind as usize]
    }
}

pub const DARK_NAME: &str = "rsdisk Dark";
pub const LIGHT_NAME: &str = "rsdisk Light";

/// The mockup's palette (`mockup/index.html`), used when a theme lacks terminal colours.
const FALLBACK_KINDS: [u32; 8] = [
    0xe06c75, 0xc678dd, 0xe5c07b, 0xd19a66, 0x61afef, 0x98c379, 0x56b6c2, 0x9aa0a6,
];

pub fn builtin_dark() -> Theme {
    Theme {
        name: DARK_NAME.into(),
        dark: true,
        bg: 0x16171a,
        surface: 0x1f2125,
        raised: 0x2c2f35,
        text: 0xe6e4df,
        dim: 0x8d9097,
        faint: 0x33363c,
        accent: 0xe5c07b,
        error: 0xe06c75,
        kinds: FALLBACK_KINDS,
    }
}

pub fn builtin_light() -> Theme {
    Theme {
        name: LIGHT_NAME.into(),
        dark: false,
        bg: 0xf6f5f2,
        surface: 0xffffff,
        raised: 0xe4e1da,
        text: 0x1d1f23,
        dim: 0x6b6f76,
        faint: 0xd9d7d2,
        accent: 0xb07d1a,
        error: 0xc4453f,
        kinds: FALLBACK_KINDS,
    }
}

/// `#rrggbb` or `#rrggbbaa`, the alpha blended over `under`.
fn colour(value: &Value, under: u32) -> Option<u32> {
    let hex = value.as_str()?.strip_prefix('#')?;
    let channel = |i: usize| u32::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let a = match hex.len() {
        6 => 255,
        8 => channel(6)?,
        _ => return None,
    };
    Some(blend((r << 16) | (g << 8) | b, under, a as f32 / 255.0))
}

/// `over` at `alpha` on top of `under`.
fn blend(over: u32, under: u32, alpha: f32) -> u32 {
    let mix = |shift: u32| {
        let o = ((over >> shift) & 0xff) as f32;
        let u = ((under >> shift) & 0xff) as f32;
        ((o * alpha + u * (1.0 - alpha)).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// One Zed theme (an entry of a theme file's `themes` list), mapped onto rsdisk's roles.
pub fn from_zed(theme: &Value) -> Option<Theme> {
    let name = theme["name"].as_str()?.to_string();
    let dark = theme["appearance"].as_str() != Some("light");
    let style = &theme["style"];
    let fallback = if dark {
        builtin_dark()
    } else {
        builtin_light()
    };
    let bg = colour(&style["background"], 0).unwrap_or(fallback.bg);
    let pick = |keys: &[&str], default: u32| {
        keys.iter()
            .find_map(|k| colour(&style[*k], bg))
            .unwrap_or(default)
    };
    let ansi = |name: &str| colour(&style[format!("terminal.ansi.{name}").as_str()], bg);
    let dim = pick(&["text.muted"], fallback.dim);
    let yellow = ansi("yellow");
    let red = ansi("red");
    // Same order as `Kind::ALL`: video, audio, image, archive, document, code, app, other.
    let kinds = [
        red,
        ansi("magenta"),
        yellow,
        ansi("bright_yellow")
            .filter(|c| Some(*c) != yellow)
            .or_else(|| Some(blend(yellow?, red?, 0.5))),
        ansi("blue"),
        ansi("green"),
        ansi("cyan"),
        Some(dim),
    ];
    let mut resolved = FALLBACK_KINDS;
    for (slot, c) in resolved.iter_mut().zip(kinds) {
        if let Some(c) = c {
            *slot = c;
        }
    }
    Some(Theme {
        name,
        dark,
        bg,
        surface: pick(&["surface.background"], fallback.surface),
        raised: pick(
            &["ghost_element.selected", "element.selected"],
            fallback.raised,
        ),
        text: pick(&["text"], fallback.text),
        dim,
        faint: pick(&["border.variant", "border"], fallback.faint),
        accent: pick(&["text.accent"], fallback.accent),
        error: pick(&["error"], fallback.error),
        kinds: resolved,
    })
}

/// Where Zed keeps installed and user themes.
pub fn zed_dirs() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut dirs = vec![home.join(".config/zed/themes")];
    let installed = home.join("Library/Application Support/Zed/extensions/installed");
    if let Ok(entries) = std::fs::read_dir(installed) {
        dirs.extend(entries.flatten().map(|e| e.path().join("themes")));
    }
    dirs
}

/// Every theme on offer: the built-ins first, then the Zed themes by name. Unreadable files
/// are skipped; a theme name seen twice keeps its first file.
pub fn available(dirs: &[PathBuf]) -> Vec<Theme> {
    let mut zed: Vec<Theme> = dirs
        .iter()
        .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .flat_map(|path| from_file(&path))
        .collect();
    zed.sort_by_key(|t| t.name.to_lowercase());
    zed.dedup_by(|a, b| a.name == b.name);
    [builtin_dark(), builtin_light()]
        .into_iter()
        .chain(zed)
        .collect()
}

/// The theme for an appearance: the one named in settings if it exists, else the built-in.
pub fn pick(themes: &[Theme], name: Option<&str>, dark: bool) -> Theme {
    name.and_then(|n| themes.iter().find(|t| t.name == n))
        .cloned()
        .unwrap_or_else(|| {
            if dark {
                builtin_dark()
            } else {
                builtin_light()
            }
        })
}

fn from_file(path: &Path) -> Vec<Theme> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|file| file["themes"].as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|t| from_zed(&t))
        .collect()
}

fn hsla(c: u32) -> Hsla {
    rgb(c).into()
}

/// Makes `theme` gpui-component's theme, so its widgets match rsdisk's own drawing.
pub fn apply(theme: &Theme, window: Option<&mut Window>, cx: &mut App) {
    let mode = if theme.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    UiTheme::change(mode, window, cx);
    let c = &mut UiTheme::global_mut(cx).colors;
    c.background = hsla(theme.bg);
    c.foreground = hsla(theme.text);
    c.muted = hsla(theme.surface);
    c.muted_foreground = hsla(theme.dim);
    c.border = hsla(theme.faint);
    c.accent = hsla(theme.raised);
    c.accent_foreground = hsla(theme.text);
    c.popover = hsla(theme.surface);
    c.popover_foreground = hsla(theme.text);
    c.list = hsla(theme.bg);
    c.list_even = hsla(theme.bg);
    c.list_hover = hsla(blend(theme.raised, theme.bg, 0.5));
    c.list_active = hsla(theme.raised);
    c.list_active_border = hsla(theme.raised);
    c.selection = hsla(blend(theme.accent, theme.bg, 0.35));
    c.danger = hsla(theme.error);
    c.scrollbar = hsla(theme.bg);
    c.scrollbar_thumb = hsla(theme.faint);
    c.title_bar = hsla(theme.surface);
    c.title_bar_border = hsla(theme.faint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tokyo() -> Value {
        json!({
            "name": "Tokyo Night",
            "appearance": "dark",
            "style": {
                "background": "#1a1b26",
                "surface.background": "#16161e",
                "element.selected": "#565f89",
                "border": "#101014",
                "text": "#a9b1d6",
                "text.muted": "#787c99",
                "text.accent": "#7dcfff",
                "error": "#f7768e",
                "terminal.ansi.red": "#f7768e",
                "terminal.ansi.magenta": "#bb9af7",
                "terminal.ansi.yellow": "#e0af68",
                "terminal.ansi.blue": "#7aa2f7",
                "terminal.ansi.green": "#9ece6a",
                "terminal.ansi.cyan": "#7dcfff"
            }
        })
    }

    #[test]
    fn maps_the_roles() {
        let t = from_zed(&tokyo()).unwrap();
        assert_eq!(t.name, "Tokyo Night");
        assert!(t.dark);
        assert_eq!(t.bg, 0x1a1b26);
        assert_eq!(t.surface, 0x16161e);
        assert_eq!(t.raised, 0x565f89);
        assert_eq!(t.accent, 0x7dcfff);
        assert_eq!(t.text, 0xa9b1d6);
        assert_eq!(t.dim, 0x787c99);
        assert_eq!(t.faint, 0x101014);
        assert_eq!(t.error, 0xf7768e);
    }

    #[test]
    fn kind_colours_come_from_the_terminal() {
        let t = from_zed(&tokyo()).unwrap();
        assert_eq!(t.kind(Kind::Video), 0xf7768e);
        assert_eq!(t.kind(Kind::Audio), 0xbb9af7);
        assert_eq!(t.kind(Kind::Image), 0xe0af68);
        assert_eq!(t.kind(Kind::Document), 0x7aa2f7);
        assert_eq!(t.kind(Kind::Code), 0x9ece6a);
        assert_eq!(t.kind(Kind::App), 0x7dcfff);
        assert_eq!(t.kind(Kind::Other), t.dim);
    }

    #[test]
    fn archive_is_bright_yellow_or_between_yellow_and_red() {
        let t = from_zed(&tokyo()).unwrap();
        assert_eq!(t.kind(Kind::Archive), blend(0xe0af68, 0xf7768e, 0.5));
        let mut theme = tokyo();
        theme["style"]["terminal.ansi.bright_yellow"] = json!("#ffc777");
        assert_eq!(from_zed(&theme).unwrap().kind(Kind::Archive), 0xffc777);
    }

    #[test]
    fn missing_kind_colours_fall_back_to_the_mockup_palette() {
        let t = from_zed(&json!({"name": "Bare", "style": {"background": "#000000"}})).unwrap();
        assert_eq!(t.kind(Kind::Video), FALLBACK_KINDS[0]);
        assert_eq!(t.kind(Kind::Archive), FALLBACK_KINDS[3]);
    }

    #[test]
    fn alpha_is_blended_over_the_background() {
        let mut theme = tokyo();
        theme["style"]["ghost_element.selected"] = json!("#ffffff80");
        let t = from_zed(&theme).unwrap();
        assert_eq!(t.raised, blend(0xffffff, 0x1a1b26, 128.0 / 255.0));
    }

    #[test]
    fn missing_keys_fall_back_to_the_builtin_of_the_same_appearance() {
        let dark = from_zed(&json!({"name": "Bare", "style": {"background": "#000000"}})).unwrap();
        assert_eq!(dark.text, builtin_dark().text);
        assert_eq!(dark.bg, 0x000000);
        let light = from_zed(&json!({"name": "Bare", "appearance": "light", "style": {}})).unwrap();
        assert!(!light.dark);
        assert_eq!(light.text, builtin_light().text);
    }

    #[test]
    fn nameless_themes_are_skipped() {
        assert_eq!(from_zed(&json!({"style": {}})), None);
    }

    #[test]
    fn every_variant_in_a_file_and_builtins_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut light = tokyo();
        light["name"] = json!("Tokyo Night Light");
        light["appearance"] = json!("light");
        std::fs::write(
            dir.path().join("tokyo.json"),
            json!({"themes": [tokyo(), light]}).to_string(),
        )
        .unwrap();
        std::fs::write(dir.path().join("broken.json"), b"{not json").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"ignored").unwrap();
        let names: Vec<String> = available(&[dir.path().into(), dir.path().join("missing")])
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(
            names,
            [DARK_NAME, LIGHT_NAME, "Tokyo Night", "Tokyo Night Light"]
        );
    }

    #[test]
    fn pick_uses_the_named_theme_or_the_builtin() {
        let themes = vec![builtin_dark(), builtin_light(), from_zed(&tokyo()).unwrap()];
        assert_eq!(pick(&themes, Some("Tokyo Night"), true).name, "Tokyo Night");
        assert_eq!(pick(&themes, Some("Gone"), true).name, DARK_NAME);
        assert_eq!(pick(&themes, None, false).name, LIGHT_NAME);
    }
}
