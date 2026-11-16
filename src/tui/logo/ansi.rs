//! Built-in & Custom Character-Art (`.ansi`/`.txt`) Sprite Loader & Panel Builder.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{extract_json_str, extract_json_u64};
use crate::tui::grid::{BoxRow, PanelSpec, TermGeom};
use crate::tui::palette::{tint_ansi_logo_line, Palette};
use crate::tui::width::display_width;
use crate::tui::RESET;

use super::{SpriteMode, SpriteStore};

pub(crate) fn parse_sprite_mode_str(s: &str) -> Option<SpriteMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(SpriteMode::Auto),
        "image" | "png" | "img" => Some(SpriteMode::Image),
        "ascii" | "ansi" | "text" | "char" => Some(SpriteMode::Ascii),
        _ => None,
    }
}

pub(crate) fn read_viewer_prefs(home: &str) -> (SpriteMode, u8) {
    let mut mode = SpriteMode::Auto;
    let mut cost_lines = 3u8;
    if let Ok(raw) = fs::read_to_string(Path::new(home).join(".config/cc-ide/viewer.json")) {
        if let Some(m) =
            extract_json_str(&raw, "sprite_mode").and_then(|s| parse_sprite_mode_str(&s))
        {
            mode = m;
        }
        if let Some(cl) = extract_json_u64(&raw, "cost_lines") {
            cost_lines = (cl as u8).clamp(1, 3);
        } else if let Some(cm) = extract_json_str(&raw, "cost_mode") {
            cost_lines = match cm.trim().to_ascii_lowercase().as_str() {
                "1" | "single" | "compact" => 1,
                "2" | "double" | "brief" => 2,
                _ => 3,
            };
        }
    }
    if let Ok(ev) = env::var("CCIDE_SPRITE_MODE") {
        if let Some(m) = parse_sprite_mode_str(&ev) {
            mode = m;
        }
    }
    if let Ok(ev) = env::var("CCIDE_COST_LINES") {
        if let Ok(cl) = ev.trim().parse::<u8>() {
            cost_lines = cl.clamp(1, 3);
        }
    }
    (mode, cost_lines)
}

pub(crate) const FAMILIES: [&str; 5] = ["claude", "gemini", "deepseek", "openai", "grok"];

const BUILTIN_CLAUDE_ANSI: &str = include_str!("../../../assets/sprites/claude.ansi");
const BUILTIN_GEMINI_ANSI: &str = include_str!("../../../assets/sprites/gemini.ansi");
const BUILTIN_DEEPSEEK_ANSI: &str = include_str!("../../../assets/sprites/deepseek.ansi");
const BUILTIN_OPENAI_ANSI: &str = include_str!("../../../assets/sprites/openai.ansi");
const BUILTIN_GROK_ANSI: &str = include_str!("../../../assets/sprites/grok.ansi");

fn builtin_ansi_for(fam: &str) -> &'static str {
    match fam {
        "claude" => BUILTIN_CLAUDE_ANSI,
        "gemini" => BUILTIN_GEMINI_ANSI,
        "deepseek" => BUILTIN_DEEPSEEK_ANSI,
        "openai" => BUILTIN_OPENAI_ANSI,
        "grok" => BUILTIN_GROK_ANSI,
        _ => BUILTIN_CLAUDE_ANSI,
    }
}

pub(crate) fn candidate_sprite_dirs(home: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !home.is_empty() {
        dirs.push(Path::new(home).join(".config/cc-ide/sprites"));
        dirs.push(Path::new(home).join(".claude/avatars"));
        dirs.push(Path::new(home).join(".cache/cc-ide/sprites"));
    }
    if let Ok(exe) = env::current_exe() {
        if let Ok(canon) = fs::canonicalize(exe) {
            if let Some(repo) = canon.parent().and_then(|p| p.parent()) {
                dirs.push(repo.join("assets/sprites"));
            }
        }
    }
    dirs.push(PathBuf::from("assets/sprites"));
    dirs
}

pub(crate) fn load_ansi_sprites(dirs: &[PathBuf]) -> HashMap<String, Vec<String>> {
    let mut ansi_map = HashMap::new();
    for fam in FAMILIES {
        let mut loaded_lines: Option<Vec<String>> = None;
        'ansi_search: for d in dirs {
            for ext in ["ansi", "txt"] {
                let p = d.join(format!("{fam}.{ext}"));
                if let Ok(txt) = fs::read_to_string(&p) {
                    let lines: Vec<String> = txt
                        .lines()
                        .filter(|l| !l.trim_start().starts_with('#'))
                        .map(String::from)
                        .collect();
                    if !lines.is_empty() {
                        loaded_lines = Some(lines);
                        break 'ansi_search;
                    }
                }
            }
        }
        let lines = loaded_lines
            .unwrap_or_else(|| builtin_ansi_for(fam).lines().map(String::from).collect());
        ansi_map.insert(fam.to_string(), lines);
    }
    ansi_map
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_ansi_logo_panel(
    geom: TermGeom,
    sprites: &SpriteStore,
    family: &str,
    badge: String,
    accent: &str,
    accent_rgb: (u8, u8, u8),
    clear_prefix: &'static str,
    pal: &Palette,
) -> (PanelSpec, usize) {
    // The leading blank row carries the one-shot Kitty deletion via `bg`, emitted verbatim.
    let mut logo_rows = vec![BoxRow {
        text: String::new(),
        bg: clear_prefix.to_string(),
        tag: None,
    }];
    if let Some(art) = sprites.ansi_map.get(family) {
        let vis_w = art.first().map(|l| display_width(l)).unwrap_or(26);
        let pad_left = geom.cols.saturating_sub(2).saturating_sub(vis_w) / 2;
        let prefix = " ".repeat(pad_left);
        for line in art {
            let rendered = if pal.tint_logo {
                tint_ansi_logo_line(line, accent_rgb)
            } else {
                line.clone()
            };
            logo_rows.push(BoxRow {
                text: format!("{}{}{RESET}", prefix, rendered),
                bg: String::new(),
                tag: None,
            });
        }
    }
    logo_rows.push(BoxRow {
        text: String::new(),
        bg: String::new(),
        tag: None,
    });
    let top_h = logo_rows.len() + 2;
    (
        PanelSpec {
            title: family.to_uppercase(),
            badge,
            accent: accent.to_string(),
            rows: logo_rows,
            fixed_height: Some(top_h),
            weight: 1.0,
            raw_interior: false,
            focused: false,
        },
        top_h,
    )
}
