//! Standalone Character-Art (`.ansi`/`.txt`) & Native Image (`.png`/`.jpg`/`.webp`/`.gif`)
//! Sprite Store & Adaptive Top Panel Builder.

pub(crate) mod ansi;
pub mod image;

use std::cell::Cell;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::tui::grid::{BoxRow, PanelSpec, TermGeom};
use crate::tui::palette::Palette;
use ansi::{
    build_ansi_logo_panel, candidate_sprite_dirs, load_ansi_sprites, read_viewer_prefs, FAMILIES,
};
use image::{
    base64_encode, build_image_blit_sequence, detect_terminal_image_proto, parse_image_dimensions,
    ImageProto, ImageSprite,
};

/// Deletes the transmitted Kitty image (`i=71`) when falling back to ASCII / `OSC 1337`.
const KITTY_CLEAR: &str = "\x1b_Ga=d,d=i,i=71,q=2\x1b\\";
/// Minimum geometry able to host an image blit inside the bordered panel.
const MIN_IMAGE_COLS: usize = 12;
const MIN_IMAGE_ROWS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteMode {
    Auto,
    Image,
    Ascii,
}

pub struct SpriteStore {
    pub(crate) ansi_map: HashMap<String, Vec<String>>,
    img_map: HashMap<String, ImageSprite>,
    detected_proto: ImageProto,
    viewer_path: PathBuf,
    last_viewer_mtime: Option<SystemTime>,
    /// Set while a Kitty image is live, so the next ASCII / `OSC 1337` frame clears it.
    was_kitty: Cell<bool>,
    pub mode: SpriteMode,
    pub cost_lines: u8,
}

pub fn load_adaptive_logos(home: &str) -> SpriteStore {
    let dirs = candidate_sprite_dirs(home);
    let ansi_map = load_ansi_sprites(&dirs);
    let mut img_map = HashMap::new();

    for fam in FAMILIES {
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
        was_kitty: Cell::new(false),
        mode,
        cost_lines,
    }
}

impl SpriteStore {
    /// One-shot Kitty image deletion when the previous frame used Kitty and this one does not.
    fn kitty_transition_prefix(&self, next: Option<ImageProto>) -> &'static str {
        let next = next == Some(ImageProto::Kitty);
        let was = self.was_kitty.replace(next);
        if was && !next {
            KITTY_CLEAR
        } else {
            ""
        }
    }

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

    // Too-small geometry falls back to ASCII. Resolve the Kitty transition once per frame.
    let img_choice = if geom.cols < MIN_IMAGE_COLS || geom.rows < MIN_IMAGE_ROWS {
        None
    } else {
        sprites.effective_proto_for(family)
    };
    let clear_prefix = sprites.kitty_transition_prefix(img_choice.map(|(p, _)| p));

    if let Some((proto, img)) = img_choice {
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
        let mut blit = build_image_blit_sequence(
            proto,
            img,
            inner_cols,
            interior_h,
            2 + pad_left,
            render_cols,
            render_rows,
        );
        if !clear_prefix.is_empty() {
            blit.insert_str(0, clear_prefix);
        }

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

    build_ansi_logo_panel(
        geom,
        sprites,
        family,
        badge,
        accent,
        accent_rgb,
        clear_prefix,
        pal,
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
