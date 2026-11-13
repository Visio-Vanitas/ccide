//! Standalone Character-Art (`.ansi`/`.txt`) & Native Image (`.png`/`.jpg`/`.webp`/`.gif`)
//! Sprite Store & Adaptive Top Panel Builder.

pub mod image;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::{extract_json_str, extract_json_u64};
use crate::tui::grid::{BoxRow, PanelSpec, TermGeom};
use crate::tui::palette::{tint_ansi_logo_line, Palette};
use crate::tui::width::display_width;
use crate::tui::RESET;
use image::{
    base64_encode, build_image_blit_sequence, detect_terminal_image_proto, parse_image_dimensions,
    ImageProto, ImageSprite,
};

const FAMILIES: [&str; 5] = ["claude", "gemini", "deepseek", "openai", "grok"];

const BUILTIN_CLAUDE_ANSI: &str = include_str!("../../../assets/sprites/claude.ansi");
const BUILTIN_GEMINI_ANSI: &str = include_str!("../../../assets/sprites/gemini.ansi");
const BUILTIN_DEEPSEEK_ANSI: &str = include_str!("../../../assets/sprites/deepseek.ansi");
const BUILTIN_OPENAI_ANSI: &str = include_str!("../../../assets/sprites/openai.ansi");
const BUILTIN_GROK_ANSI: &str = include_str!("../../../assets/sprites/grok.ansi");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteMode {
    Auto,
    Image,
    Ascii,
}

pub struct SpriteStore {
    ansi_map: HashMap<String, Vec<String>>,
    img_map: HashMap<String, ImageSprite>,
    detected_proto: ImageProto,
    viewer_path: PathBuf,
    last_viewer_mtime: Option<SystemTime>,
    pub mode: SpriteMode,
    pub cost_lines: u8,
}

fn parse_sprite_mode_str(s: &str) -> Option<SpriteMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(SpriteMode::Auto),
        "image" | "png" | "img" => Some(SpriteMode::Image),
        "ascii" | "ansi" | "text" | "char" => Some(SpriteMode::Ascii),
        _ => None,
    }
}

fn read_viewer_prefs(home: &str) -> (SpriteMode, u8) {
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

fn candidate_sprite_dirs(home: &str) -> Vec<PathBuf> {
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

pub fn load_adaptive_logos(home: &str) -> SpriteStore {
    let dirs = candidate_sprite_dirs(home);
    let mut ansi_map = HashMap::new();
    let mut img_map = HashMap::new();

    for fam in FAMILIES {
        let mut loaded_lines: Option<Vec<String>> = None;
        'ansi_search: for d in &dirs {
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

        'img_search: for d in &dirs {
            for ext in ["png", "jpg", "jpeg", "webp", "gif"] {
                let p = d.join(format!("{fam}.{ext}"));
                if let Ok(meta) = fs::metadata(&p) {
                    if meta.is_file() && meta.len() > 0 && meta.len() <= 2 * 1024 * 1024 {
                        if let Ok(bytes) = fs::read(&p) {
                            let (w, h) = parse_image_dimensions(&bytes);
                            img_map.insert(
                                fam.to_string(),
                                ImageSprite {
                                    ext: ext.to_string(),
                                    width_px: w,
                                    height_px: h,
                                    b64: base64_encode(&bytes),
                                },
                            );
                            break 'img_search;
                        }
                    }
                }
            }
        }
    }

    let viewer_path = Path::new(home).join(".config/cc-ide/viewer.json");
    let last_viewer_mtime = fs::metadata(&viewer_path).and_then(|m| m.modified()).ok();

    let (mode, cost_lines) = read_viewer_prefs(home);
    SpriteStore {
        ansi_map,
        img_map,
        detected_proto: detect_terminal_image_proto(),
        viewer_path,
        last_viewer_mtime,
        mode,
        cost_lines,
    }
}

impl SpriteStore {
    pub fn refresh_mode_if_changed(&mut self, home: &str) -> bool {
        let cur_mt = fs::metadata(&self.viewer_path)
            .and_then(|m| m.modified())
            .ok();
        if cur_mt != self.last_viewer_mtime {
            self.last_viewer_mtime = cur_mt;
            let (new_mode, new_cl) = read_viewer_prefs(home);
            let changed = new_mode != self.mode || new_cl != self.cost_lines;
            self.mode = new_mode;
            self.cost_lines = new_cl;
            return changed;
        }
        false
    }

    fn effective_proto_for(&self, fam: &str) -> Option<(ImageProto, &ImageSprite)> {
        let img = self.img_map.get(fam)?;
        let proto = match self.mode {
            SpriteMode::Ascii => return None,
            SpriteMode::Auto => self.detected_proto,
            SpriteMode::Image => match self.detected_proto {
                ImageProto::None => ImageProto::Osc1337,
                other => other,
            },
        };
        match proto {
            ImageProto::None => None,
            ImageProto::Osc1337 => Some((ImageProto::Osc1337, img)),
            ImageProto::Kitty if img.ext == "png" => Some((ImageProto::Kitty, img)),
            ImageProto::Kitty => {
                let term_prog = env::var("TERM_PROGRAM").unwrap_or_default();
                if term_prog == "iTerm.app"
                    || env::var("LC_TERMINAL").as_deref() == Ok("iTerm2")
                    || term_prog == "WezTerm"
                {
                    Some((ImageProto::Osc1337, img))
                } else {
                    None
                }
            }
        }
    }
}

pub fn build_top_logo_panel(
    geom: TermGeom,
    sprites: &SpriteStore,
    family: &str,
    badge: String,
    pal: &Palette,
) -> (PanelSpec, usize) {
    let (accent, accent_rgb) = pal.family_accent(family);

    if let Some((proto, img)) = sprites.effective_proto_for(family) {
        let inner_cols = geom.cols.saturating_sub(2).max(10);
        let render_rows = (geom.rows / 5).clamp(8, 11);
        let aspect = (img.width_px.max(1) as f64) / (img.height_px.max(1) as f64);
        let cell_w = if geom.cell_w_px > 1.0 {
            geom.cell_w_px
        } else {
            14.0
        };
        let cell_h = if geom.cell_h_px > 1.0 {
            geom.cell_h_px
        } else {
            30.0
        };
        let ideal_cols = (((render_rows as f64) * cell_h * aspect) / cell_w).round() as usize;
        let render_cols = ideal_cols.clamp(8, inner_cols.saturating_sub(2).max(8));
        let pad_left = inner_cols.saturating_sub(render_cols) / 2;

        let interior_h = render_rows + 2;
        let top_h = interior_h + 2;
        let blit = build_image_blit_sequence(
            proto,
            img,
            inner_cols,
            interior_h,
            2 + pad_left,
            render_cols,
            render_rows,
        );

        let mut raw_rows = Vec::with_capacity(interior_h);
        raw_rows.push(BoxRow {
            text: blit,
            bg: String::new(),
            tag: None,
        });
        for _ in 1..interior_h {
            raw_rows.push(BoxRow {
                text: String::new(),
                bg: String::new(),
                tag: None,
            });
        }
        return (
            PanelSpec {
                title: family.to_uppercase(),
                badge,
                accent: accent.to_string(),
                rows: raw_rows,
                fixed_height: Some(top_h),
                weight: 1.0,
                raw_interior: true,
                focused: false,
            },
            top_h,
        );
    }

    let mut logo_rows = vec![BoxRow {
        text: String::new(),
        bg: String::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_standalone_ansi_and_png_sprites() {
        assert_eq!(base64_encode(b"ccide"), "Y2NpZGU=");
        let claude_png = include_bytes!("../../../assets/sprites/claude.png");
        assert_eq!(parse_image_dimensions(claude_png), (256, 256));

        let mut store = load_adaptive_logos("/nonexistent");
        let pal = Palette::default_palette();
        let geom = TermGeom {
            rows: 56,
            cols: 38,
            cell_w_px: 14.0,
            cell_h_px: 30.0,
        };
        store.mode = SpriteMode::Ascii;
        let (p_ascii, _) =
            build_top_logo_panel(geom, &store, "gemini", "gemini-4".to_string(), &pal);
        assert!(!p_ascii.raw_interior);
    }
}
