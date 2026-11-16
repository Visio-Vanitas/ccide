//! JSONC Comment Stripper, Monochrome Logo Tinter & Palette Template Generator.

use super::presets::RawThemeColors;
use super::{parse_color_spec, ColorSpec};
use crate::json::{extract_json_str, extract_json_u64};

pub(crate) fn extract_color_field(clean_json: &str, key: &str) -> Option<ColorSpec> {
    extract_json_str(clean_json, key)
        .and_then(|s| parse_color_spec(&s))
        .or_else(|| {
            extract_json_u64(clean_json, key)
                .filter(|&n| n <= 255)
                .map(|n| ColorSpec::Ansi256(n as u8))
        })
}

pub use crate::json::strip_jsonc as strip_jsonc_comments;

pub(crate) fn ansi256_to_rgb(n: u8) -> (u8, u8, u8) {
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
    const BRIGHT: [(u8, u8, u8); 8] = [
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];
    match n {
        0..=7 => BASE[n as usize],
        8..=15 => BRIGHT[(n - 8) as usize],
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

/// Optional monochrome logo tinter (`"tint_logo": true` in `palette.json`):
/// maps every `\x1b[38;2;R;G;Bm` sequence in a logo line to `target_rgb` scaled by perceived luminance.
pub fn tint_ansi_logo_line(line: &str, target_rgb: (u8, u8, u8)) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut rest = line;
    while let Some(pos) = rest.find("\x1b[38;2;") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 7..];
        if let Some(m_idx) = after.find('m') {
            let mut nums = after[..m_idx].split(';');
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
  "ui": {{
    // "border":   "#444A5E",  "divider":  "#303446",  "grid_dot": "#2D3244",
    // "text":     "#E8ECF8",  "dim":      "#787E96",  "cyan":     "#56D6FF",
    // "green":    "#5CE49A",  "amber":    "#FFC25C",  "coral":    "#EE7C64",
    // "blue":     "#62A4FF",  "mint":     "#38DAAC",  "lavender": "#C08EFF"
  }},

  "highlights": {{
    // "active_view_bg": "#203E5C", "cursor_bg": "#36304E", "active_cursor_bg": "#2A5276"
  }},

  "models": {{
    // "model_claude": "#EE7C64", "model_gemini": "#62A4FF", "model_deepseek": "#56D6FF",
    // "model_openai": "#38DAAC", "model_grok":   "#C08EFF"
  }},

  "syntax": {{
    // "syn_keyword":     "#C08EFF", "syn_function":   "#62A4FF", "syn_type":     "#FFC25C",
    // "syn_string":      "#5CE49A", "syn_number":     "#FFC25C", "syn_comment":  "#787E96",
    // "syn_operator":    "#56D6FF", "syn_variable":   "#E8ECF8", "syn_code_bg":  "#1E222E",
    // "syn_diff_add_fg": "#5CE49A", "syn_diff_add_bg": "#1A3327",
    // "syn_diff_del_fg": "#EE7C64", "syn_diff_del_bg": "#382022"
  }},

  "statusline": {{
    // "sl_bypass_bg": 131,  "sl_bypass_fg": 255,  "sl_auto_bg":   61,   "sl_auto_fg":   255,
    // "sl_plan_bg":   30,   "sl_plan_fg":   255,  "sl_accept_bg": 29,   "sl_accept_fg": 255,
    // "sl_manual_bg": 239,  "sl_manual_fg": 252,  "sl_model_bg":  24,   "sl_model_fg":  255,
    // "sl_ctx_ok_bg": 240,  "sl_ctx_ok_fg": 253,  "sl_ctx_warn_bg": 136,"sl_ctx_warn_fg": 255,
    // "sl_ctx_crit_bg": 131,"sl_ctx_crit_fg": 255,
    // "sl_git_bg":    237,  "sl_git_fg":    250,  "sl_sys_bg":    235,  "sl_sys_fg":    247
  }}
}}
"##
    )
}
