use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use gpui_kit::component::{Theme, ThemeMode, try_parse_color};
use gpui_kit::{App, Hsla};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Zh,
    En,
}

impl Language {
    pub fn pick<'a>(self, zh: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Zh => zh,
            Self::En => en,
        }
    }

    pub fn is_zh(self) -> bool {
        matches!(self, Self::Zh)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThemeSettings {
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub foreground: Option<String>,
    #[serde(default)]
    pub sidebar: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub theme: ThemeSettings,
    #[serde(default)]
    pub notes_root: Option<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: Language::Zh,
            theme: ThemeSettings::default(),
            notes_root: None,
        }
    }
}

impl AppSettings {
    pub fn config_path() -> PathBuf {
        if let Ok(value) = std::env::var("WHITEBOARD_CONFIG")
            && !value.trim().is_empty()
        {
            return PathBuf::from(value);
        }

        dirs::config_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("WhiteboardNotes")
            .join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        match fs::read_to_string(&path) {
            Ok(source) => serde_json::from_str(&source).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }
        let source = serde_json::to_string_pretty(self)
            .context("Failed to serialize application settings")?;
        fs::write(&path, source)
            .with_context(|| format!("Failed to save application settings: {}", path.display()))
    }

    pub fn effective_notes_root(&self) -> PathBuf {
        if let Some(root) = &self.notes_root {
            return root.clone();
        }
        crate::model::data_root()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThemePreset {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub background: Option<&'static str>,
    pub foreground: Option<&'static str>,
    pub sidebar: Option<&'static str>,
    pub border: Option<&'static str>,
    pub accent: Option<&'static str>,
}

pub const THEME_PRESETS: &[ThemePreset] = &[
    ThemePreset {
        id: "default-light",
        name: "Default Light",
        dark: false,
        background: None,
        foreground: None,
        sidebar: None,
        border: None,
        accent: None,
    },
    ThemePreset {
        id: "default-dark",
        name: "Default Dark",
        dark: true,
        background: None,
        foreground: None,
        sidebar: None,
        border: None,
        accent: None,
    },
    ThemePreset {
        id: "ocean",
        name: "Ocean",
        dark: true,
        background: Some("#0B1220"),
        foreground: Some("#DCE7F5"),
        sidebar: Some("#101B31"),
        border: Some("#223252"),
        accent: Some("#38BDF8"),
    },
    ThemePreset {
        id: "forest",
        name: "Forest",
        dark: false,
        background: Some("#F2F7F1"),
        foreground: Some("#1F2A1E"),
        sidebar: Some("#E4EFE1"),
        border: Some("#C5D8C1"),
        accent: Some("#2F855A"),
    },
    ThemePreset {
        id: "sunset",
        name: "Sunset",
        dark: false,
        background: Some("#FFF7ED"),
        foreground: Some("#3B1F0B"),
        sidebar: Some("#FFEDD5"),
        border: Some("#FED7AA"),
        accent: Some("#EA580C"),
    },
    ThemePreset {
        id: "rose",
        name: "Rose",
        dark: false,
        background: Some("#FFF1F2"),
        foreground: Some("#3B0A16"),
        sidebar: Some("#FFE4E6"),
        border: Some("#FECDD3"),
        accent: Some("#E11D48"),
    },
    ThemePreset {
        id: "nord",
        name: "Nord",
        dark: true,
        background: Some("#2E3440"),
        foreground: Some("#ECEFF4"),
        sidebar: Some("#3B4252"),
        border: Some("#4C566A"),
        accent: Some("#88C0D0"),
    },
    ThemePreset {
        id: "solarized",
        name: "Solarized",
        dark: false,
        background: Some("#FDF6E3"),
        foreground: Some("#586E75"),
        sidebar: Some("#EEE8D5"),
        border: Some("#D6D0C4"),
        accent: Some("#268BD2"),
    },
    ThemePreset {
        id: "cyberpunk",
        name: "Cyberpunk",
        dark: true,
        background: Some("#0A0014"),
        foreground: Some("#E8E0FF"),
        sidebar: Some("#16002B"),
        border: Some("#4C1D95"),
        accent: Some("#D946EF"),
    },
    ThemePreset {
        id: "paper",
        name: "Paper",
        dark: false,
        background: Some("#FAF8F3"),
        foreground: Some("#2B2A26"),
        sidebar: Some("#F0ECE2"),
        border: Some("#D9D2C3"),
        accent: Some("#7C5C3B"),
    },
];

impl ThemeSettings {
    pub fn matches_preset(&self, preset: &ThemePreset) -> bool {
        self.dark == preset.dark
            && option_matches(&self.background, preset.background)
            && option_matches(&self.foreground, preset.foreground)
            && option_matches(&self.sidebar, preset.sidebar)
            && option_matches(&self.border, preset.border)
            && option_matches(&self.accent, preset.accent)
    }

    pub fn apply_preset(&mut self, preset: &ThemePreset) {
        self.dark = preset.dark;
        self.background = preset.background.map(str::to_string);
        self.foreground = preset.foreground.map(str::to_string);
        self.sidebar = preset.sidebar.map(str::to_string);
        self.border = preset.border.map(str::to_string);
        self.accent = preset.accent.map(str::to_string);
    }
}

fn option_matches(value: &Option<String>, expected: Option<&str>) -> bool {
    match (value.as_deref(), expected) {
        (None, None) => true,
        (Some(value), Some(expected)) => value.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

pub fn apply_theme(settings: &ThemeSettings, cx: &mut App) {
    let mode = if settings.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, None, cx);

    let accent = settings
        .accent
        .as_deref()
        .and_then(|value| try_parse_color(value).ok());
    let background = settings
        .background
        .as_deref()
        .and_then(|value| try_parse_color(value).ok());
    let foreground = settings
        .foreground
        .as_deref()
        .and_then(|value| try_parse_color(value).ok());
    let sidebar = settings
        .sidebar
        .as_deref()
        .and_then(|value| try_parse_color(value).ok());
    let border = settings
        .border
        .as_deref()
        .and_then(|value| try_parse_color(value).ok());

    if background.is_some() || foreground.is_some() || sidebar.is_some() || border.is_some() {
        let current = Theme::global(cx).colors;
        let base_background = background.unwrap_or(current.background);
        let base_foreground = foreground.unwrap_or(current.foreground);
        let accent = accent.unwrap_or(current.primary);
        let accent_foreground = contrast_color(accent);
        let sidebar_background =
            sidebar.unwrap_or_else(|| mix(base_background, base_foreground, 0.03));
        let border_color = border.unwrap_or_else(|| {
            mix(
                base_background,
                base_foreground,
                if settings.dark { 0.26 } else { 0.18 },
            )
        });
        let popover = mix(base_background, base_foreground, 0.04);
        let muted = mix(base_background, base_foreground, 0.07);
        let muted_foreground = mix(base_foreground, base_background, 0.38);
        let input = mix(base_background, base_foreground, 0.08);
        let secondary = mix(base_background, base_foreground, 0.09);
        let list_hover = mix(base_background, base_foreground, 0.08);

        Theme::update(cx, |theme| {
            theme.colors.background = base_background;
            theme.colors.foreground = base_foreground;
            theme.colors.popover = popover;
            theme.colors.popover_foreground = base_foreground;
            theme.colors.sidebar = sidebar_background;
            theme.colors.sidebar_foreground = base_foreground;
            theme.colors.sidebar_border = border_color;
            theme.colors.sidebar_accent = accent;
            theme.colors.sidebar_accent_foreground = accent_foreground;
            theme.colors.border = border_color;
            theme.colors.input = input;
            theme.colors.muted = muted;
            theme.colors.muted_foreground = muted_foreground;
            theme.colors.secondary = secondary;
            theme.colors.secondary_foreground = base_foreground;
            theme.colors.accent = mix(base_background, accent, 0.12);
            theme.colors.accent_foreground = base_foreground;
            theme.colors.list = popover;
            theme.colors.list_hover = list_hover;
            theme.colors.list_even = popover;
            theme.colors.list_active = accent;
            theme.colors.list_active_border = accent;
            theme.colors.title_bar = mix(base_background, base_foreground, 0.03);
            theme.colors.status_bar = mix(base_background, base_foreground, 0.03);
            theme.colors.status_bar_border = border_color;
            theme.colors.caret = base_foreground;
            theme.colors.scrollbar = mix(base_background, base_foreground, 0.04);
            theme.colors.scrollbar_thumb = mix(base_background, base_foreground, 0.20);
            theme.colors.scrollbar_thumb_hover = mix(base_background, base_foreground, 0.30);
            theme.colors.button = mix(base_background, base_foreground, 0.06);
            theme.colors.button_hover = mix(base_background, base_foreground, 0.10);
            theme.colors.button_active = mix(base_background, base_foreground, 0.14);
            theme.colors.button_foreground = base_foreground;
        });
    }

    let Some(accent) = accent else {
        return;
    };
    let foreground = contrast_color(accent);
    let hover = Hsla {
        l: (accent.l + 0.06).min(1.0),
        ..accent
    };
    let active = Hsla {
        l: (accent.l - 0.06).max(0.0),
        ..accent
    };

    Theme::update(cx, |theme| {
        theme.colors.primary = accent;
        theme.colors.primary_foreground = foreground;
        theme.colors.primary_hover = hover;
        theme.colors.primary_active = active;
        theme.colors.button_primary = accent;
        theme.colors.button_primary_foreground = foreground;
        theme.colors.button_primary_hover = hover;
        theme.colors.button_primary_active = active;
        theme.colors.sidebar_primary = accent;
        theme.colors.sidebar_primary_foreground = foreground;
        theme.colors.ring = accent;
        theme.colors.link = accent;
        theme.colors.link_hover = hover;
        theme.colors.link_active = active;
    });
}

fn contrast_color(color: Hsla) -> Hsla {
    if color.l < 0.58 {
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 1.0,
        }
    } else {
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.08,
            a: 1.0,
        }
    }
}

fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let amount = amount.clamp(0.0, 1.0);
    Hsla {
        h: from.h + (to.h - from.h) * amount,
        s: from.s + (to.s - from.s) * amount,
        l: from.l + (to.l - from.l) * amount,
        a: 1.0,
    }
}

pub fn accent_preview(hex: &str) -> Option<Hsla> {
    try_parse_color(hex).ok()
}

pub fn validate_accent(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("Color is empty");
    }
    try_parse_color(value).with_context(|| format!("Invalid color: {value}"))?;
    Ok(value.to_string())
}

pub fn optional_color(value: &str) -> Result<Option<String>> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        validate_accent(value).map(Some)
    }
}

pub fn set_notes_root_default() -> PathBuf {
    crate::model::data_root()
}

pub fn path_is_usable(path: &Path) -> bool {
    if path.exists() {
        path.is_dir()
    } else {
        fs::create_dir_all(path).is_ok()
    }
}
