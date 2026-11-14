//! User-Customizable TUI, Syntax & Powerline Color Palette (`~/.config/cc-ide/palette.json`).

pub(crate) mod artifacts;
pub(crate) mod companions;
pub mod presets;
pub mod syntax;
pub mod template;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use presets::{StatuslinePalette, PRESET_NAMES};
pub use syntax::{sync_syntax_artifacts, SyntaxPalette};
pub use template::{render_palette_template, strip_jsonc_comments, tint_ansi_logo_line};

pub(crate) use presets::hex_nib;
use presets::UiColorSpecs;
pub(crate) use template::extract_color_field;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpec {
    Rgb(u8, u8, u8),
    Ansi256(u8),
}

impl ColorSpec {
    pub fn fg_ansi(self) -> String {
        match self {
            Self::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
            Self::Ansi256(n) => format!("\x1b[38;5;{n}m"),
        }
    }
    pub fn bg_ansi(self) -> String {
        match self {
            Self::Rgb(r, g, b) => format!("\x1b[48;2;{r};{g};{b}m"),
            Self::Ansi256(n) => format!("\x1b[48;5;{n}m"),
        }
    }
    pub fn to_rgb_approx(self) -> (u8, u8, u8) {
        match self {
            Self::Rgb(r, g, b) => (r, g, b),
            Self::Ansi256(n) => ansi256_to_rgb(n),
        }
    }
    pub fn to_hex_string(self) -> String {
        let (r, g, b) = self.to_rgb_approx();
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

fn ansi256_to_rgb(n: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 8] = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
    ];
    match n {
        0..=7 => BASE[n as usize],
        8..=15 => (180, 180, 180),
        16..=231 => {
            let i = n - 16;
            let c = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (c(i / 36), c((i % 36) / 6), c(i % 6))
        }
        232..=255 => {
            let v = 8 + (n - 232) * 10;
            (v, v, v)
        }
    }
}

/// Parses `"#RRGGBB"`, `"#RGB"`, `"rgb(r, g, b)"`, `"r, g, b"`, or `"0".."255"`.
pub fn parse_color_spec(raw: &str) -> Option<ColorSpec> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let hb = hex.as_bytes();
        if hb.len() == 6 {
            let b = |i| Some((hex_nib(hb[i])? << 4) | hex_nib(hb[i + 1])?);
            return Some(ColorSpec::Rgb(b(0)?, b(2)?, b(4)?));
        } else if hb.len() == 3 {
            let n = |i| Some(hex_nib(hb[i])? * 17);
            return Some(ColorSpec::Rgb(n(0)?, n(1)?, n(2)?));
        }
        return None;
    }
    let inner = s
        .strip_prefix("rgb(")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(s);
    if inner.contains(',') {
        let mut p = inner.split(',');
        let (r, g, b) = (
            p.next()?.trim().parse().ok()?,
            p.next()?.trim().parse().ok()?,
            p.next()?.trim().parse().ok()?,
        );
        return p.next().is_none().then_some(ColorSpec::Rgb(r, g, b));
    }
    s.parse::<u16>()
        .ok()
        .filter(|&i| i <= 255)
        .map(|i| ColorSpec::Ansi256(i as u8))
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub preset: String,
    pub tint_logo: bool,
    pub border: String,
    pub divider: String,
    pub grid_dot: String,
    pub text: String,
    pub dim: String,
    pub cyan: String,
    pub green: String,
    pub amber: String,
    pub coral: String,
    pub blue: String,
    pub mint: String,
    pub lavender: String,
    pub active_view_bg: String,
    pub cursor_bg: String,
    pub active_cursor_bg: String,
    pub model_claude: String,
    pub model_claude_rgb: (u8, u8, u8),
    pub model_gemini: String,
    pub model_gemini_rgb: (u8, u8, u8),
    pub model_deepseek: String,
    pub model_deepseek_rgb: (u8, u8, u8),
    pub model_openai: String,
    pub model_openai_rgb: (u8, u8, u8),
    pub model_grok: String,
    pub model_grok_rgb: (u8, u8, u8),
    pub statusline: StatuslinePalette,
    pub syntax: SyntaxPalette,
    pub(crate) ui_specs: UiColorSpecs,
}

/// Resolves the active palette configuration file path (`~/.config/cc-ide/palette.json` by default).
pub fn resolve_palette_path(home: &str) -> PathBuf {
    let non_empty = |k| {
        env::var(k)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(e) = non_empty("CCIDE_PALETTE_FILE") {
        return PathBuf::from(e);
    }
    if let Some(x) = non_empty("XDG_CONFIG_HOME") {
        let p = Path::new(&x).join("cc-ide/palette.json");
        if p.exists() {
            return p;
        }
    }
    let primary = Path::new(home).join(".config/cc-ide/palette.json");
    let alt = Path::new(home).join(".claude/ccide-palette.json");
    if primary.exists() || !alt.exists() {
        primary
    } else {
        alt
    }
}

/// Lightweight mtime-based hot-reload watcher for `ccide-core sidebar`.
pub struct PaletteWatcher {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    pub palette: Palette,
}

impl PaletteWatcher {
    pub fn new(home: &str) -> Self {
        let path = resolve_palette_path(home);
        let last_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        let palette = Palette::load(home);
        sync_syntax_artifacts(home, &palette);
        Self {
            path,
            last_mtime,
            palette,
        }
    }

    pub fn reload_if_changed(&mut self, home: &str) -> bool {
        let new_path = resolve_palette_path(home);
        let cur_mtime = fs::metadata(&new_path).and_then(|m| m.modified()).ok();
        if new_path == self.path && cur_mtime == self.last_mtime {
            return false;
        }
        self.path = new_path;
        self.last_mtime = cur_mtime;
        self.palette = Palette::load(home);
        sync_syntax_artifacts(home, &self.palette);
        crate::commands::workspace::actions::refresh_open_yazi_previews(home);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors_presets_and_syncs_syntax_artifacts() {
        let c = parse_color_spec("#56D6FF").unwrap();
        assert_eq!(
            (c, c.to_hex_string().as_str()),
            (ColorSpec::Rgb(86, 214, 255), "#56D6FF")
        );
        assert_eq!(
            parse_color_spec("#abc"),
            Some(ColorSpec::Rgb(0xaa, 0xbb, 0xcc))
        );
        assert_eq!(
            parse_color_spec("rgb(32, 62, 92)"),
            Some(ColorSpec::Rgb(32, 62, 92))
        );
        assert_eq!(
            (parse_color_spec("131"), parse_color_spec("#ZZZZZZ")),
            (Some(ColorSpec::Ansi256(131)), None)
        );
        for name in PRESET_NAMES {
            assert_eq!(
                Palette::from_json_str(&render_palette_template(name)).preset,
                name
            );
        }
        let src = r#"{"note": "C:\用户", "preset": "nord"}"#;
        assert_eq!(strip_jsonc_comments(src), src);

        let dir = env::temp_dir().join(format!("ccide_pal_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let pf = dir.join("palette.json");
        fs::write(
            &pf,
            r##"{"preset":"nord","syntax":{"syn_keyword":"#FF00AA","syn_diff_add_bg":"#102015"}}"##,
        )
        .unwrap();
        env::set_var("CCIDE_PALETTE_FILE", &pf);
        let pal = Palette::load(dir.to_str().unwrap());
        assert_eq!(
            (pal.syntax.keyword, pal.syntax.function),
            (ColorSpec::Rgb(255, 0, 170), ColorSpec::Rgb(136, 192, 208))
        );
        sync_syntax_artifacts(dir.to_str().unwrap(), &pal);
        let res = fs::read_to_string(dir.join("syntax_resolved.json")).unwrap();
        assert!(
            res.contains("\"keyword\": \"#FF00AA\"")
                && res.contains("\"diff_add_emph_bg\": \"#20402A\"")
        );
        assert!(
            dir.join("ccide.tmTheme").is_file()
                && dir.join("glow-style.json").is_file()
                && dir.join("ghostty-theme.conf").is_file()
        );
        env::remove_var("CCIDE_PALETTE_FILE");
        let _ = fs::remove_dir_all(&dir);
    }
}
