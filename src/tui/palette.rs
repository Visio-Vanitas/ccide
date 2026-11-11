//! User-Customizable TUI & Powerline Color Palette (`~/.config/cc-ide/palette.json`).
//!
//! Resolution order:
//! 1. `$CCIDE_PALETTE_FILE` (explicit file path override)
//! 2. `$XDG_CONFIG_HOME/cc-ide/palette.json`
//! 3. `~/.config/cc-ide/palette.json` (seeded from `assets/palette.json` on install)
//! 4. `~/.claude/ccide-palette.json`
//!
//! Supports JSON / JSONC (`//` and `/* ... */` comments), 6 built-in named presets
//! (`default`, `catppuccin-mocha`, `tokyo-night`, `nord`, `gruvbox-dark`, `dracula`),
//! and granular per-slot overrides using `#RRGGBB`, `#RGB`, `rgb(r,g,b)`, or ANSI 256-color
//! indices (`0..=255`). Hot-reloads live in `ccide-core sidebar` when the file's mtime changes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::{extract_json_str, extract_json_u64};
use crate::tui::RESET;

pub const PRESET_NAMES: [&str; 6] = [
    "default",
    "catppuccin-mocha",
    "tokyo-night",
    "nord",
    "gruvbox-dark",
    "dracula",
];

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
}

fn ansi256_to_rgb(n: u8) -> (u8, u8, u8) {
    match n {
        0 => (0, 0, 0),
        1 => (205, 49, 49),
        2 => (13, 188, 121),
        3 => (229, 229, 16),
        4 => (36, 114, 200),
        5 => (188, 63, 188),
        6 => (17, 168, 205),
        7 => (229, 229, 229),
        8..=15 => (180, 180, 180),
        16..=231 => {
            let idx = n - 16;
            let r = idx / 36;
            let g = (idx % 36) / 6;
            let b = idx % 6;
            let map_c = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (map_c(r), map_c(g), map_c(b))
        }
        232..=255 => {
            let v = 8 + (n - 232) * 10;
            (v, v, v)
        }
    }
}

fn parse_hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some((parse_hex_nibble(hi)? << 4) | parse_hex_nibble(lo)?)
}

/// Parses `"#RRGGBB"`, `"#RGB"`, `"rgb(r, g, b)"`, `"r, g, b"`, or `"0".."255"`.
pub fn parse_color_spec(raw: &str) -> Option<ColorSpec> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix('#') {
        let hb = hex.as_bytes();
        if hb.len() == 6 {
            let r = parse_hex_byte(hb[0], hb[1])?;
            let g = parse_hex_byte(hb[2], hb[3])?;
            let b = parse_hex_byte(hb[4], hb[5])?;
            return Some(ColorSpec::Rgb(r, g, b));
        } else if hb.len() == 3 {
            let r = parse_hex_nibble(hb[0])? * 17;
            let g = parse_hex_nibble(hb[1])? * 17;
            let b = parse_hex_nibble(hb[2])? * 17;
            return Some(ColorSpec::Rgb(r, g, b));
        }
        return None;
    }
    let inner = s
        .strip_prefix("rgb(")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(s);
    if inner.contains(',') {
        let mut parts = inner.split(',');
        let r = parts.next()?.trim().parse::<u8>().ok()?;
        let g = parts.next()?.trim().parse::<u8>().ok()?;
        let b = parts.next()?.trim().parse::<u8>().ok()?;
        if parts.next().is_none() {
            return Some(ColorSpec::Rgb(r, g, b));
        }
        return None;
    }
    if let Ok(idx) = s.parse::<u16>() {
        if idx <= 255 {
            return Some(ColorSpec::Ansi256(idx as u8));
        }
    }
    None
}

/// Strips `//` line comments, `/* ... */` block comments, and leading `#` line comments
/// outside JSON string literals so `~/.config/cc-ide/palette.json` supports JSONC comments.
pub fn strip_jsonc_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0usize;
    let mut in_str = false;
    let mut line_start_ws = true;

    while i < len {
        let b = bytes[i];
        if in_str {
            if b == b'\\' && i + 1 < len {
                out.push(b as char);
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            if let Some(ch) = src[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
            continue;
        }

        if b == b'"' {
            in_str = true;
            line_start_ws = false;
            out.push('"');
            i += 1;
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }

        if line_start_ws && b == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'\n' {
            line_start_ws = true;
            out.push('\n');
            i += 1;
            continue;
        }

        if !b.is_ascii_whitespace() {
            line_start_ws = false;
        }

        if let Some(ch) = src[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    out
}

fn extract_json_bool(json: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{}\"", key);
    let mut search = json;
    while let Some(pos) = search.find(&needle) {
        let after = &search[pos + needle.len()..];
        let trimmed = after.trim_start();
        if let Some(rest) = trimmed.strip_prefix(':') {
            let val = rest.trim_start();
            if val.starts_with("true") {
                return Some(true);
            }
            if val.starts_with("false") {
                return Some(false);
            }
        }
        search = after;
    }
    None
}

fn extract_color_field(clean_json: &str, key: &str) -> Option<ColorSpec> {
    if let Some(s) = extract_json_str(clean_json, key) {
        if let Some(c) = parse_color_spec(&s) {
            return Some(c);
        }
    }
    if let Some(n) = extract_json_u64(clean_json, key) {
        if n <= 255 {
            return Some(ColorSpec::Ansi256(n as u8));
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct StatuslinePalette {
    pub bypass_bg: ColorSpec,
    pub bypass_fg: ColorSpec,
    pub auto_bg: ColorSpec,
    pub auto_fg: ColorSpec,
    pub plan_bg: ColorSpec,
    pub plan_fg: ColorSpec,
    pub accept_bg: ColorSpec,
    pub accept_fg: ColorSpec,
    pub manual_bg: ColorSpec,
    pub manual_fg: ColorSpec,
    pub model_bg: ColorSpec,
    pub model_fg: ColorSpec,
    pub ctx_ok_bg: ColorSpec,
    pub ctx_ok_fg: ColorSpec,
    pub ctx_warn_bg: ColorSpec,
    pub ctx_warn_fg: ColorSpec,
    pub ctx_crit_bg: ColorSpec,
    pub ctx_crit_fg: ColorSpec,
    pub git_bg: ColorSpec,
    pub git_fg: ColorSpec,
    pub sys_bg: ColorSpec,
    pub sys_fg: ColorSpec,
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub preset: String,
    pub tint_logo: bool,
    // Pre-formatted ANSI foreground escapes for fast zero-overhead TUI rendering
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
    // Pre-formatted ANSI background escapes for row highlights
    pub active_view_bg: String,
    pub cursor_bg: String,
    pub active_cursor_bg: String,
    // Model family accents (and RGB tuples for optional monochrome logo tinting)
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
    // Bottom Powerline statusline segment specs
    pub statusline: StatuslinePalette,
}

#[derive(Clone, Copy)]
struct RawThemeColors {
    border: ColorSpec,
    divider: ColorSpec,
    grid_dot: ColorSpec,
    text: ColorSpec,
    dim: ColorSpec,
    cyan: ColorSpec,
    green: ColorSpec,
    amber: ColorSpec,
    coral: ColorSpec,
    blue: ColorSpec,
    mint: ColorSpec,
    lavender: ColorSpec,
    active_view_bg: ColorSpec,
    cursor_bg: ColorSpec,
    active_cursor_bg: ColorSpec,
    sl_bypass_bg: ColorSpec,
    sl_bypass_fg: ColorSpec,
    sl_auto_bg: ColorSpec,
    sl_auto_fg: ColorSpec,
    sl_plan_bg: ColorSpec,
    sl_plan_fg: ColorSpec,
    sl_accept_bg: ColorSpec,
    sl_accept_fg: ColorSpec,
    sl_manual_bg: ColorSpec,
    sl_manual_fg: ColorSpec,
    sl_model_bg: ColorSpec,
    sl_model_fg: ColorSpec,
    sl_ctx_ok_bg: ColorSpec,
    sl_ctx_ok_fg: ColorSpec,
    sl_ctx_warn_bg: ColorSpec,
    sl_ctx_warn_fg: ColorSpec,
    sl_ctx_crit_bg: ColorSpec,
    sl_ctx_crit_fg: ColorSpec,
    sl_git_bg: ColorSpec,
    sl_git_fg: ColorSpec,
    sl_sys_bg: ColorSpec,
    sl_sys_fg: ColorSpec,
}

impl RawThemeColors {
    fn from_preset(name: &str) -> (&'static str, Self) {
        match name.trim().to_ascii_lowercase().as_str() {
            "catppuccin-mocha" | "catppuccin" | "mocha" => (
                "catppuccin-mocha",
                Self {
                    border: ColorSpec::Rgb(88, 91, 112),
                    divider: ColorSpec::Rgb(69, 71, 90),
                    grid_dot: ColorSpec::Rgb(49, 50, 68),
                    text: ColorSpec::Rgb(205, 214, 244),
                    dim: ColorSpec::Rgb(147, 153, 178),
                    cyan: ColorSpec::Rgb(137, 220, 235),
                    green: ColorSpec::Rgb(166, 227, 161),
                    amber: ColorSpec::Rgb(249, 226, 175),
                    coral: ColorSpec::Rgb(243, 139, 168),
                    blue: ColorSpec::Rgb(137, 180, 250),
                    mint: ColorSpec::Rgb(148, 226, 213),
                    lavender: ColorSpec::Rgb(203, 166, 247),
                    active_view_bg: ColorSpec::Rgb(49, 50, 68),
                    cursor_bg: ColorSpec::Rgb(69, 58, 92),
                    active_cursor_bg: ColorSpec::Rgb(69, 71, 90),
                    sl_bypass_bg: ColorSpec::Rgb(210, 15, 57),
                    sl_bypass_fg: ColorSpec::Rgb(239, 241, 245),
                    sl_auto_bg: ColorSpec::Rgb(136, 57, 239),
                    sl_auto_fg: ColorSpec::Rgb(239, 241, 245),
                    sl_plan_bg: ColorSpec::Rgb(32, 159, 181),
                    sl_plan_fg: ColorSpec::Rgb(239, 241, 245),
                    sl_accept_bg: ColorSpec::Rgb(64, 160, 43),
                    sl_accept_fg: ColorSpec::Rgb(239, 241, 245),
                    sl_manual_bg: ColorSpec::Rgb(88, 91, 112),
                    sl_manual_fg: ColorSpec::Rgb(205, 214, 244),
                    sl_model_bg: ColorSpec::Rgb(30, 102, 245),
                    sl_model_fg: ColorSpec::Rgb(239, 241, 245),
                    sl_ctx_ok_bg: ColorSpec::Rgb(69, 71, 90),
                    sl_ctx_ok_fg: ColorSpec::Rgb(205, 214, 244),
                    sl_ctx_warn_bg: ColorSpec::Rgb(223, 142, 29),
                    sl_ctx_warn_fg: ColorSpec::Rgb(30, 30, 46),
                    sl_ctx_crit_bg: ColorSpec::Rgb(210, 15, 57),
                    sl_ctx_crit_fg: ColorSpec::Rgb(239, 241, 245),
                    sl_git_bg: ColorSpec::Rgb(49, 50, 68),
                    sl_git_fg: ColorSpec::Rgb(186, 194, 222),
                    sl_sys_bg: ColorSpec::Rgb(24, 24, 37),
                    sl_sys_fg: ColorSpec::Rgb(166, 173, 200),
                },
            ),
            "tokyo-night" | "tokyonight" => (
                "tokyo-night",
                Self {
                    border: ColorSpec::Rgb(65, 72, 104),
                    divider: ColorSpec::Rgb(52, 59, 88),
                    grid_dot: ColorSpec::Rgb(41, 46, 66),
                    text: ColorSpec::Rgb(192, 202, 245),
                    dim: ColorSpec::Rgb(115, 122, 162),
                    cyan: ColorSpec::Rgb(125, 207, 255),
                    green: ColorSpec::Rgb(158, 206, 106),
                    amber: ColorSpec::Rgb(224, 175, 104),
                    coral: ColorSpec::Rgb(247, 118, 142),
                    blue: ColorSpec::Rgb(122, 162, 247),
                    mint: ColorSpec::Rgb(115, 218, 202),
                    lavender: ColorSpec::Rgb(187, 154, 247),
                    active_view_bg: ColorSpec::Rgb(40, 52, 87),
                    cursor_bg: ColorSpec::Rgb(54, 48, 86),
                    active_cursor_bg: ColorSpec::Rgb(51, 70, 124),
                    sl_bypass_bg: ColorSpec::Rgb(219, 75, 75),
                    sl_bypass_fg: ColorSpec::Rgb(247, 250, 255),
                    sl_auto_bg: ColorSpec::Rgb(157, 124, 216),
                    sl_auto_fg: ColorSpec::Rgb(247, 250, 255),
                    sl_plan_bg: ColorSpec::Rgb(26, 188, 156),
                    sl_plan_fg: ColorSpec::Rgb(26, 27, 38),
                    sl_accept_bg: ColorSpec::Rgb(115, 218, 202),
                    sl_accept_fg: ColorSpec::Rgb(26, 27, 38),
                    sl_manual_bg: ColorSpec::Rgb(65, 72, 104),
                    sl_manual_fg: ColorSpec::Rgb(192, 202, 245),
                    sl_model_bg: ColorSpec::Rgb(61, 89, 161),
                    sl_model_fg: ColorSpec::Rgb(247, 250, 255),
                    sl_ctx_ok_bg: ColorSpec::Rgb(59, 66, 97),
                    sl_ctx_ok_fg: ColorSpec::Rgb(192, 202, 245),
                    sl_ctx_warn_bg: ColorSpec::Rgb(224, 175, 104),
                    sl_ctx_warn_fg: ColorSpec::Rgb(26, 27, 38),
                    sl_ctx_crit_bg: ColorSpec::Rgb(219, 75, 75),
                    sl_ctx_crit_fg: ColorSpec::Rgb(247, 250, 255),
                    sl_git_bg: ColorSpec::Rgb(41, 46, 66),
                    sl_git_fg: ColorSpec::Rgb(169, 177, 214),
                    sl_sys_bg: ColorSpec::Rgb(31, 35, 53),
                    sl_sys_fg: ColorSpec::Rgb(139, 147, 186),
                },
            ),
            "nord" => (
                "nord",
                Self {
                    border: ColorSpec::Rgb(76, 86, 106),
                    divider: ColorSpec::Rgb(67, 76, 94),
                    grid_dot: ColorSpec::Rgb(59, 66, 82),
                    text: ColorSpec::Rgb(236, 239, 244),
                    dim: ColorSpec::Rgb(144, 154, 174),
                    cyan: ColorSpec::Rgb(136, 192, 208),
                    green: ColorSpec::Rgb(163, 190, 140),
                    amber: ColorSpec::Rgb(235, 203, 139),
                    coral: ColorSpec::Rgb(191, 97, 106),
                    blue: ColorSpec::Rgb(129, 161, 193),
                    mint: ColorSpec::Rgb(143, 188, 187),
                    lavender: ColorSpec::Rgb(180, 142, 173),
                    active_view_bg: ColorSpec::Rgb(59, 66, 82),
                    cursor_bg: ColorSpec::Rgb(67, 76, 94),
                    active_cursor_bg: ColorSpec::Rgb(76, 86, 106),
                    sl_bypass_bg: ColorSpec::Rgb(191, 97, 106),
                    sl_bypass_fg: ColorSpec::Rgb(236, 239, 244),
                    sl_auto_bg: ColorSpec::Rgb(180, 142, 173),
                    sl_auto_fg: ColorSpec::Rgb(46, 52, 64),
                    sl_plan_bg: ColorSpec::Rgb(136, 192, 208),
                    sl_plan_fg: ColorSpec::Rgb(46, 52, 64),
                    sl_accept_bg: ColorSpec::Rgb(163, 190, 140),
                    sl_accept_fg: ColorSpec::Rgb(46, 52, 64),
                    sl_manual_bg: ColorSpec::Rgb(76, 86, 106),
                    sl_manual_fg: ColorSpec::Rgb(236, 239, 244),
                    sl_model_bg: ColorSpec::Rgb(94, 129, 172),
                    sl_model_fg: ColorSpec::Rgb(236, 239, 244),
                    sl_ctx_ok_bg: ColorSpec::Rgb(67, 76, 94),
                    sl_ctx_ok_fg: ColorSpec::Rgb(229, 233, 240),
                    sl_ctx_warn_bg: ColorSpec::Rgb(208, 135, 112),
                    sl_ctx_warn_fg: ColorSpec::Rgb(46, 52, 64),
                    sl_ctx_crit_bg: ColorSpec::Rgb(191, 97, 106),
                    sl_ctx_crit_fg: ColorSpec::Rgb(236, 239, 244),
                    sl_git_bg: ColorSpec::Rgb(59, 66, 82),
                    sl_git_fg: ColorSpec::Rgb(216, 222, 233),
                    sl_sys_bg: ColorSpec::Rgb(46, 52, 64),
                    sl_sys_fg: ColorSpec::Rgb(180, 188, 202),
                },
            ),
            "gruvbox-dark" | "gruvbox" => (
                "gruvbox-dark",
                Self {
                    border: ColorSpec::Rgb(102, 92, 84),
                    divider: ColorSpec::Rgb(80, 73, 69),
                    grid_dot: ColorSpec::Rgb(60, 56, 54),
                    text: ColorSpec::Rgb(235, 219, 178),
                    dim: ColorSpec::Rgb(168, 153, 132),
                    cyan: ColorSpec::Rgb(131, 165, 152),
                    green: ColorSpec::Rgb(184, 187, 38),
                    amber: ColorSpec::Rgb(250, 189, 47),
                    coral: ColorSpec::Rgb(251, 73, 52),
                    blue: ColorSpec::Rgb(69, 133, 136),
                    mint: ColorSpec::Rgb(142, 192, 124),
                    lavender: ColorSpec::Rgb(211, 134, 155),
                    active_view_bg: ColorSpec::Rgb(60, 56, 54),
                    cursor_bg: ColorSpec::Rgb(80, 73, 69),
                    active_cursor_bg: ColorSpec::Rgb(102, 92, 84),
                    sl_bypass_bg: ColorSpec::Rgb(204, 36, 29),
                    sl_bypass_fg: ColorSpec::Rgb(251, 241, 199),
                    sl_auto_bg: ColorSpec::Rgb(177, 98, 134),
                    sl_auto_fg: ColorSpec::Rgb(251, 241, 199),
                    sl_plan_bg: ColorSpec::Rgb(104, 157, 106),
                    sl_plan_fg: ColorSpec::Rgb(40, 40, 40),
                    sl_accept_bg: ColorSpec::Rgb(152, 151, 26),
                    sl_accept_fg: ColorSpec::Rgb(40, 40, 40),
                    sl_manual_bg: ColorSpec::Rgb(80, 73, 69),
                    sl_manual_fg: ColorSpec::Rgb(235, 219, 178),
                    sl_model_bg: ColorSpec::Rgb(69, 133, 136),
                    sl_model_fg: ColorSpec::Rgb(251, 241, 199),
                    sl_ctx_ok_bg: ColorSpec::Rgb(80, 73, 69),
                    sl_ctx_ok_fg: ColorSpec::Rgb(235, 219, 178),
                    sl_ctx_warn_bg: ColorSpec::Rgb(215, 153, 33),
                    sl_ctx_warn_fg: ColorSpec::Rgb(40, 40, 40),
                    sl_ctx_crit_bg: ColorSpec::Rgb(204, 36, 29),
                    sl_ctx_crit_fg: ColorSpec::Rgb(251, 241, 199),
                    sl_git_bg: ColorSpec::Rgb(60, 56, 54),
                    sl_git_fg: ColorSpec::Rgb(213, 196, 161),
                    sl_sys_bg: ColorSpec::Rgb(40, 40, 40),
                    sl_sys_fg: ColorSpec::Rgb(168, 153, 132),
                },
            ),
            "dracula" => (
                "dracula",
                Self {
                    border: ColorSpec::Rgb(98, 114, 164),
                    divider: ColorSpec::Rgb(68, 71, 90),
                    grid_dot: ColorSpec::Rgb(52, 55, 70),
                    text: ColorSpec::Rgb(248, 248, 242),
                    dim: ColorSpec::Rgb(139, 148, 184),
                    cyan: ColorSpec::Rgb(139, 233, 253),
                    green: ColorSpec::Rgb(80, 250, 123),
                    amber: ColorSpec::Rgb(241, 250, 140),
                    coral: ColorSpec::Rgb(255, 85, 85),
                    blue: ColorSpec::Rgb(98, 114, 164),
                    mint: ColorSpec::Rgb(80, 250, 123),
                    lavender: ColorSpec::Rgb(189, 147, 249),
                    active_view_bg: ColorSpec::Rgb(68, 71, 90),
                    cursor_bg: ColorSpec::Rgb(78, 66, 110),
                    active_cursor_bg: ColorSpec::Rgb(88, 92, 130),
                    sl_bypass_bg: ColorSpec::Rgb(255, 85, 85),
                    sl_bypass_fg: ColorSpec::Rgb(248, 248, 242),
                    sl_auto_bg: ColorSpec::Rgb(189, 147, 249),
                    sl_auto_fg: ColorSpec::Rgb(40, 42, 54),
                    sl_plan_bg: ColorSpec::Rgb(139, 233, 253),
                    sl_plan_fg: ColorSpec::Rgb(40, 42, 54),
                    sl_accept_bg: ColorSpec::Rgb(80, 250, 123),
                    sl_accept_fg: ColorSpec::Rgb(40, 42, 54),
                    sl_manual_bg: ColorSpec::Rgb(68, 71, 90),
                    sl_manual_fg: ColorSpec::Rgb(248, 248, 242),
                    sl_model_bg: ColorSpec::Rgb(98, 114, 164),
                    sl_model_fg: ColorSpec::Rgb(248, 248, 242),
                    sl_ctx_ok_bg: ColorSpec::Rgb(68, 71, 90),
                    sl_ctx_ok_fg: ColorSpec::Rgb(248, 248, 242),
                    sl_ctx_warn_bg: ColorSpec::Rgb(255, 184, 108),
                    sl_ctx_warn_fg: ColorSpec::Rgb(40, 42, 54),
                    sl_ctx_crit_bg: ColorSpec::Rgb(255, 85, 85),
                    sl_ctx_crit_fg: ColorSpec::Rgb(248, 248, 242),
                    sl_git_bg: ColorSpec::Rgb(52, 55, 70),
                    sl_git_fg: ColorSpec::Rgb(248, 248, 242),
                    sl_sys_bg: ColorSpec::Rgb(40, 42, 54),
                    sl_sys_fg: ColorSpec::Rgb(189, 195, 218),
                },
            ),
            _ => (
                "default",
                Self {
                    border: ColorSpec::Rgb(68, 74, 94),
                    divider: ColorSpec::Rgb(48, 52, 70),
                    grid_dot: ColorSpec::Rgb(45, 50, 68),
                    text: ColorSpec::Rgb(232, 236, 248),
                    dim: ColorSpec::Rgb(120, 126, 150),
                    cyan: ColorSpec::Rgb(86, 214, 255),
                    green: ColorSpec::Rgb(92, 228, 154),
                    amber: ColorSpec::Rgb(255, 194, 92),
                    coral: ColorSpec::Rgb(238, 124, 100),
                    blue: ColorSpec::Rgb(98, 164, 255),
                    mint: ColorSpec::Rgb(56, 218, 172),
                    lavender: ColorSpec::Rgb(192, 142, 255),
                    active_view_bg: ColorSpec::Rgb(32, 62, 92),
                    cursor_bg: ColorSpec::Rgb(54, 48, 78),
                    active_cursor_bg: ColorSpec::Rgb(42, 82, 118),
                    sl_bypass_bg: ColorSpec::Ansi256(131),
                    sl_bypass_fg: ColorSpec::Ansi256(255),
                    sl_auto_bg: ColorSpec::Ansi256(61),
                    sl_auto_fg: ColorSpec::Ansi256(255),
                    sl_plan_bg: ColorSpec::Ansi256(30),
                    sl_plan_fg: ColorSpec::Ansi256(255),
                    sl_accept_bg: ColorSpec::Ansi256(29),
                    sl_accept_fg: ColorSpec::Ansi256(255),
                    sl_manual_bg: ColorSpec::Ansi256(239),
                    sl_manual_fg: ColorSpec::Ansi256(252),
                    sl_model_bg: ColorSpec::Ansi256(24),
                    sl_model_fg: ColorSpec::Ansi256(255),
                    sl_ctx_ok_bg: ColorSpec::Ansi256(240),
                    sl_ctx_ok_fg: ColorSpec::Ansi256(253),
                    sl_ctx_warn_bg: ColorSpec::Ansi256(136),
                    sl_ctx_warn_fg: ColorSpec::Ansi256(255),
                    sl_ctx_crit_bg: ColorSpec::Ansi256(131),
                    sl_ctx_crit_fg: ColorSpec::Ansi256(255),
                    sl_git_bg: ColorSpec::Ansi256(237),
                    sl_git_fg: ColorSpec::Ansi256(250),
                    sl_sys_bg: ColorSpec::Ansi256(235),
                    sl_sys_fg: ColorSpec::Ansi256(247),
                },
            ),
        }
    }
}

/// Resolves the active palette configuration file path (`~/.config/cc-ide/palette.json` by default).
pub fn resolve_palette_path(home: &str) -> PathBuf {
    if let Ok(explicit) = env::var("CCIDE_PALETTE_FILE") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            let p = Path::new(trimmed).join("cc-ide/palette.json");
            if p.exists() {
                return p;
            }
        }
    }
    let primary = Path::new(home).join(".config/cc-ide/palette.json");
    if primary.exists() {
        return primary;
    }
    let alt = Path::new(home).join(".claude/ccide-palette.json");
    if alt.exists() {
        return alt;
    }
    primary
}

impl Palette {
    pub fn default_palette() -> Self {
        Self::from_json_str("")
    }

    pub fn from_json_str(raw_src: &str) -> Self {
        let clean = strip_jsonc_comments(raw_src);
        let req_preset = extract_json_str(&clean, "preset").unwrap_or_default();
        let (canonical_preset, mut c) = RawThemeColors::from_preset(&req_preset);
        let tint_logo = extract_json_bool(&clean, "tint_logo").unwrap_or(false);

        macro_rules! override_slot {
            ($field:ident, $key:expr) => {
                if let Some(v) = extract_color_field(&clean, $key) {
                    c.$field = v;
                }
            };
        }

        override_slot!(border, "border");
        override_slot!(divider, "divider");
        override_slot!(grid_dot, "grid_dot");
        override_slot!(text, "text");
        override_slot!(dim, "dim");
        override_slot!(cyan, "cyan");
        override_slot!(green, "green");
        override_slot!(amber, "amber");
        override_slot!(coral, "coral");
        override_slot!(blue, "blue");
        override_slot!(mint, "mint");
        override_slot!(lavender, "lavender");

        override_slot!(active_view_bg, "active_view_bg");
        override_slot!(cursor_bg, "cursor_bg");
        override_slot!(active_cursor_bg, "active_cursor_bg");

        override_slot!(sl_bypass_bg, "sl_bypass_bg");
        override_slot!(sl_bypass_fg, "sl_bypass_fg");
        override_slot!(sl_auto_bg, "sl_auto_bg");
        override_slot!(sl_auto_fg, "sl_auto_fg");
        override_slot!(sl_plan_bg, "sl_plan_bg");
        override_slot!(sl_plan_fg, "sl_plan_fg");
        override_slot!(sl_accept_bg, "sl_accept_bg");
        override_slot!(sl_accept_fg, "sl_accept_fg");
        override_slot!(sl_manual_bg, "sl_manual_bg");
        override_slot!(sl_manual_fg, "sl_manual_fg");
        override_slot!(sl_model_bg, "sl_model_bg");
        override_slot!(sl_model_fg, "sl_model_fg");
        override_slot!(sl_ctx_ok_bg, "sl_ctx_ok_bg");
        override_slot!(sl_ctx_ok_fg, "sl_ctx_ok_fg");
        override_slot!(sl_ctx_warn_bg, "sl_ctx_warn_bg");
        override_slot!(sl_ctx_warn_fg, "sl_ctx_warn_fg");
        override_slot!(sl_ctx_crit_bg, "sl_ctx_crit_bg");
        override_slot!(sl_ctx_crit_fg, "sl_ctx_crit_fg");
        override_slot!(sl_git_bg, "sl_git_bg");
        override_slot!(sl_git_fg, "sl_git_fg");
        override_slot!(sl_sys_bg, "sl_sys_bg");
        override_slot!(sl_sys_fg, "sl_sys_fg");

        let m_claude = extract_color_field(&clean, "model_claude").unwrap_or(c.coral);
        let m_gemini = extract_color_field(&clean, "model_gemini").unwrap_or(c.blue);
        let m_deepseek = extract_color_field(&clean, "model_deepseek").unwrap_or(c.cyan);
        let m_openai = extract_color_field(&clean, "model_openai").unwrap_or(c.mint);
        let m_grok = extract_color_field(&clean, "model_grok").unwrap_or(c.lavender);

        Self {
            preset: canonical_preset.to_string(),
            tint_logo,
            border: c.border.fg_ansi(),
            divider: c.divider.fg_ansi(),
            grid_dot: format!("{}·{}", c.grid_dot.fg_ansi(), RESET),
            text: c.text.fg_ansi(),
            dim: c.dim.fg_ansi(),
            cyan: c.cyan.fg_ansi(),
            green: c.green.fg_ansi(),
            amber: c.amber.fg_ansi(),
            coral: c.coral.fg_ansi(),
            blue: c.blue.fg_ansi(),
            mint: c.mint.fg_ansi(),
            lavender: c.lavender.fg_ansi(),
            active_view_bg: c.active_view_bg.bg_ansi(),
            cursor_bg: c.cursor_bg.bg_ansi(),
            active_cursor_bg: c.active_cursor_bg.bg_ansi(),
            model_claude: m_claude.fg_ansi(),
            model_claude_rgb: m_claude.to_rgb_approx(),
            model_gemini: m_gemini.fg_ansi(),
            model_gemini_rgb: m_gemini.to_rgb_approx(),
            model_deepseek: m_deepseek.fg_ansi(),
            model_deepseek_rgb: m_deepseek.to_rgb_approx(),
            model_openai: m_openai.fg_ansi(),
            model_openai_rgb: m_openai.to_rgb_approx(),
            model_grok: m_grok.fg_ansi(),
            model_grok_rgb: m_grok.to_rgb_approx(),
            statusline: StatuslinePalette {
                bypass_bg: c.sl_bypass_bg,
                bypass_fg: c.sl_bypass_fg,
                auto_bg: c.sl_auto_bg,
                auto_fg: c.sl_auto_fg,
                plan_bg: c.sl_plan_bg,
                plan_fg: c.sl_plan_fg,
                accept_bg: c.sl_accept_bg,
                accept_fg: c.sl_accept_fg,
                manual_bg: c.sl_manual_bg,
                manual_fg: c.sl_manual_fg,
                model_bg: c.sl_model_bg,
                model_fg: c.sl_model_fg,
                ctx_ok_bg: c.sl_ctx_ok_bg,
                ctx_ok_fg: c.sl_ctx_ok_fg,
                ctx_warn_bg: c.sl_ctx_warn_bg,
                ctx_warn_fg: c.sl_ctx_warn_fg,
                ctx_crit_bg: c.sl_ctx_crit_bg,
                ctx_crit_fg: c.sl_ctx_crit_fg,
                git_bg: c.sl_git_bg,
                git_fg: c.sl_git_fg,
                sys_bg: c.sl_sys_bg,
                sys_fg: c.sl_sys_fg,
            },
        }
    }

    pub fn load(home: &str) -> Self {
        let p = resolve_palette_path(home);
        match fs::read_to_string(&p) {
            Ok(txt) => Self::from_json_str(&txt),
            Err(_) => Self::default_palette(),
        }
    }

    pub fn family_accent(&self, family: &str) -> (&str, (u8, u8, u8)) {
        match family {
            "gemini" => (&self.model_gemini, self.model_gemini_rgb),
            "deepseek" => (&self.model_deepseek, self.model_deepseek_rgb),
            "openai" => (&self.model_openai, self.model_openai_rgb),
            "grok" => (&self.model_grok, self.model_grok_rgb),
            _ => (&self.model_claude, self.model_claude_rgb),
        }
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
        Self {
            path,
            last_mtime,
            palette,
        }
    }

    /// Checks whether the palette file changed on disk (`1` stat syscall, `0` reads when unchanged).
    /// Returns `true` if reloaded so the caller can invalidate its row diff cache.
    pub fn reload_if_changed(&mut self, home: &str) -> bool {
        let new_path = resolve_palette_path(home);
        let cur_mtime = fs::metadata(&new_path).and_then(|m| m.modified()).ok();
        if new_path != self.path || cur_mtime != self.last_mtime {
            self.path = new_path;
            self.last_mtime = cur_mtime;
            self.palette = Palette::load(home);
            return true;
        }
        false
    }
}

/// Optional monochrome logo tinter (`"tint_logo": true` in `palette.json`):
/// maps every `\x1b[38;2;R;G;Bm` sequence in a logo line to `target_rgb` scaled by perceived luminance.
pub fn tint_ansi_logo_line(line: &str, target_rgb: (u8, u8, u8)) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut rest = line;
    while let Some(pos) = rest.find("\x1b[38;2;") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 7..];
        if let Some(m_idx) = after.find('m') {
            let body = &after[..m_idx];
            let mut nums = body.split(';');
            if let (Some(rs), Some(gs), Some(bs)) = (nums.next(), nums.next(), nums.next()) {
                if let (Ok(r), Ok(g), Ok(b)) =
                    (rs.parse::<f64>(), gs.parse::<f64>(), bs.parse::<f64>())
                {
                    let lum = ((0.299 * r + 0.587 * g + 0.114 * b) / 255.0).clamp(0.18, 1.0);
                    let nr = ((target_rgb.0 as f64) * lum).round().clamp(0.0, 255.0) as u8;
                    let ng = ((target_rgb.1 as f64) * lum).round().clamp(0.0, 255.0) as u8;
                    let nb = ((target_rgb.2 as f64) * lum).round().clamp(0.0, 255.0) as u8;
                    out.push_str(&format!("\x1b[38;2;{nr};{ng};{nb}m"));
                    rest = &after[m_idx + 1..];
                    continue;
                }
            }
        }
        out.push_str("\x1b[38;2;");
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Generates a clean, commented `~/.config/cc-ide/palette.json` configuration file for `preset`.
pub fn render_palette_template(preset: &str) -> String {
    let (canon, _) = RawThemeColors::from_preset(preset);
    format!(
        r##"{{
  // Active theme preset: "default" | "catppuccin-mocha" | "tokyo-night" | "nord" | "gruvbox-dark" | "dracula"
  "preset": "{canon}",

  // Set to true to monochrome-tint the top vector model logo with the active family's accent color
  "tint_logo": false,

  // Optional per-slot overrides (accepts "#RRGGBB", "#RGB", "rgb(r,g,b)", or ANSI 256-color 0..255).
  // Uncomment any line below to customize individual colors on top of the selected preset:
  "ui": {{
    // "border":   "#444A5E",
    // "divider":  "#303446",
    // "grid_dot": "#2D3244",
    // "text":     "#E8ECF8",
    // "dim":      "#787E96",
    // "cyan":     "#56D6FF",
    // "green":    "#5CE49A",
    // "amber":    "#FFC25C",
    // "coral":    "#EE7C64",
    // "blue":     "#62A4FF",
    // "mint":     "#38DAAC",
    // "lavender": "#C08EFF"
  }},

  "highlights": {{
    // "active_view_bg":   "#203E5C",
    // "cursor_bg":        "#36304E",
    // "active_cursor_bg": "#2A5276"
  }},

  "models": {{
    // "model_claude":   "#EE7C64",
    // "model_gemini":   "#62A4FF",
    // "model_deepseek": "#56D6FF",
    // "model_openai":   "#38DAAC",
    // "model_grok":     "#C08EFF"
  }},

  "statusline": {{
    // "sl_bypass_bg": 131,  "sl_bypass_fg": 255,
    // "sl_auto_bg":   61,   "sl_auto_fg":   255,
    // "sl_plan_bg":   30,   "sl_plan_fg":   255,
    // "sl_accept_bg": 29,   "sl_accept_fg": 255,
    // "sl_manual_bg": 239,  "sl_manual_fg": 252,
    // "sl_model_bg":  24,   "sl_model_fg":  255,
    // "sl_ctx_ok_bg": 240,  "sl_ctx_ok_fg": 253,
    // "sl_ctx_warn_bg": 136,"sl_ctx_warn_fg": 255,
    // "sl_ctx_crit_bg": 131,"sl_ctx_crit_fg": 255,
    // "sl_git_bg":    237,  "sl_git_fg":    250,
    // "sl_sys_bg":    235,  "sl_sys_fg":    247
  }}
}}
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_rgb_and_ansi256_specs() {
        assert_eq!(
            parse_color_spec("#56D6FF"),
            Some(ColorSpec::Rgb(86, 214, 255))
        );
        assert_eq!(
            parse_color_spec("#abc"),
            Some(ColorSpec::Rgb(0xaa, 0xbb, 0xcc))
        );
        assert_eq!(
            parse_color_spec("rgb(32, 62, 92)"),
            Some(ColorSpec::Rgb(32, 62, 92))
        );
        assert_eq!(parse_color_spec("131"), Some(ColorSpec::Ansi256(131)));
        assert_eq!(parse_color_spec("#ZZZZZZ"), None);
        assert_eq!(parse_color_spec("999"), None);
    }

    #[test]
    fn jsonc_comments_preserve_hex_strings_and_apply_overrides() {
        let cfg = r##"
        {
          // Choose Nord base preset
          "preset": "nord",
          "tint_logo": true,
          "ui": {
            "cyan": "#112233", /* custom cyan override */
            // "amber": "#999999" (commented out -> stays Nord amber!)
          },
          "statusline": {
            "sl_model_bg": 61
          }
        }
        "##;
        let p = Palette::from_json_str(cfg);
        assert_eq!(p.preset, "nord");
        assert!(p.tint_logo);
        assert_eq!(p.cyan, "\x1b[38;2;17;34;51m");
        // Nord amber (#EBCB8B = 235, 203, 139) survives commented-out override
        assert_eq!(p.amber, "\x1b[38;2;235;203;139m");
        assert_eq!(p.statusline.model_bg, ColorSpec::Ansi256(61));
    }

    #[test]
    fn all_named_presets_load_distinct_palettes() {
        for name in PRESET_NAMES {
            let tpl = render_palette_template(name);
            let p = Palette::from_json_str(&tpl);
            assert_eq!(p.preset, name);
        }
    }
}
