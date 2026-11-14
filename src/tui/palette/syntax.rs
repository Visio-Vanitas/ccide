//! Palette-Unified Syntax Highlighting Model & Atomic Sync (`<= 300` lines).

use std::fs;
use std::path::Path;

use crate::json::extract_json_str;

use super::artifacts::{build_ghostty_theme, build_glow_style_json, build_tm_theme};
use super::companions::sync_companion_themes;
use super::{resolve_palette_path, ColorSpec, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxPalette {
    pub keyword: ColorSpec,
    pub function: ColorSpec,
    pub type_name: ColorSpec,
    pub string: ColorSpec,
    pub number: ColorSpec,
    pub comment: ColorSpec,
    pub operator: ColorSpec,
    pub variable: ColorSpec,
    pub code_bg: ColorSpec,
    pub diff_add_fg: ColorSpec,
    pub diff_add_bg: ColorSpec,
    pub diff_del_fg: ColorSpec,
    pub diff_del_bg: ColorSpec,
}

impl SyntaxPalette {
    pub(crate) fn apply_overrides(&mut self, clean: &str) {
        const KEYS: &str = "syn_keyword syn_function syn_type syn_string syn_number syn_comment syn_operator syn_variable syn_code_bg syn_diff_add_fg syn_diff_add_bg syn_diff_del_fg syn_diff_del_bg";
        let mut fields = [
            &mut self.keyword,
            &mut self.function,
            &mut self.type_name,
            &mut self.string,
            &mut self.number,
            &mut self.comment,
            &mut self.operator,
            &mut self.variable,
            &mut self.code_bg,
            &mut self.diff_add_fg,
            &mut self.diff_add_bg,
            &mut self.diff_del_fg,
            &mut self.diff_del_bg,
        ];
        for (slot, k) in fields.iter_mut().zip(KEYS.split_ascii_whitespace()) {
            if let Some(v) = super::extract_color_field(clean, k) {
                **slot = v;
            }
        }
    }
}

fn sgr(c: ColorSpec, prefix: u8) -> String {
    let (r, g, b) = c.to_rgb_approx();
    format!("{prefix};2;{r};{g};{b}")
}

fn emph_bg_hex(c: ColorSpec) -> String {
    let (r, g, b) = c.to_rgb_approx();
    format!(
        "#{:02X}{:02X}{:02X}",
        r.saturating_mul(2),
        g.saturating_mul(2),
        b.saturating_mul(2)
    )
}

fn compute_digest(pal: &Palette) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in b"v2:" {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    for &b in pal.preset.as_bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let s = &pal.syntax;
    let syn_all = [
        s.keyword,
        s.function,
        s.type_name,
        s.string,
        s.number,
        s.comment,
        s.operator,
        s.variable,
        s.code_bg,
        s.diff_add_fg,
        s.diff_add_bg,
        s.diff_del_fg,
        s.diff_del_bg,
    ];
    for c in pal.ui_specs.iter().chain(syn_all.iter()) {
        let (r, g, b) = c.to_rgb_approx();
        for x in [r, g, b] {
            h = (h ^ x as u64).wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}

pub(crate) fn atomic_write(path: &Path, content: &str) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("art");
    let tmp = path.with_file_name(format!("{name}.tmp.{}", std::process::id()));
    if fs::write(&tmp, content).is_ok() && fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        if fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

fn build_syntax_resolved_json(pal: &Palette, digest: &str, bat_theme: &str) -> String {
    let (u, s) = (&pal.ui_specs, &pal.syntax);
    let hex_pairs: [(&str, String); 25] = [
        ("text", u[0].to_hex_string()),
        ("dim", u[1].to_hex_string()),
        ("border", u[2].to_hex_string()),
        ("cyan", u[3].to_hex_string()),
        ("blue", u[4].to_hex_string()),
        ("lavender", u[5].to_hex_string()),
        ("amber", u[6].to_hex_string()),
        ("green", u[7].to_hex_string()),
        ("coral", u[8].to_hex_string()),
        ("mint", u[9].to_hex_string()),
        ("keyword", s.keyword.to_hex_string()),
        ("function", s.function.to_hex_string()),
        ("type_name", s.type_name.to_hex_string()),
        ("string", s.string.to_hex_string()),
        ("number", s.number.to_hex_string()),
        ("comment", s.comment.to_hex_string()),
        ("operator", s.operator.to_hex_string()),
        ("variable", s.variable.to_hex_string()),
        ("code_bg", s.code_bg.to_hex_string()),
        ("diff_add_fg", s.diff_add_fg.to_hex_string()),
        ("diff_add_bg", s.diff_add_bg.to_hex_string()),
        ("diff_add_emph_bg", emph_bg_hex(s.diff_add_bg)),
        ("diff_del_fg", s.diff_del_fg.to_hex_string()),
        ("diff_del_bg", s.diff_del_bg.to_hex_string()),
        ("diff_del_emph_bg", emph_bg_hex(s.diff_del_bg)),
    ];
    let sgr_pairs: [(&str, String); 12] = [
        ("31", sgr(s.variable, 38)),
        ("32", sgr(s.string, 38)),
        ("33", sgr(s.type_name, 38)),
        ("34", sgr(s.function, 38)),
        ("35", sgr(s.keyword, 38)),
        ("36", sgr(s.operator, 38)),
        ("37", sgr(u[0], 38)),
        ("38;5;8", sgr(s.comment, 38)),
        ("38;5;9", sgr(s.number, 38)),
        ("38;5;16", sgr(s.number, 38)),
        ("38;5;17", sgr(s.operator, 38)),
        ("code_bg", sgr(s.code_bg, 48)),
    ];
    let fmt_map = |items: &[(&str, String)]| -> String {
        items
            .iter()
            .enumerate()
            .map(|(i, (k, v))| {
                let c = if i + 1 < items.len() { "," } else { "" };
                format!("    \"{k}\": \"{v}\"{c}\n")
            })
            .collect()
    };
    format!(
        "{{\n  \"digest\": \"{digest}\", \"preset\": \"{}\", \"bat_theme\": \"{bat_theme}\",\n  \"hex\": {{\n{}  }},\n  \"sgr\": {{\n{}  }}\n}}\n",
        pal.preset,
        fmt_map(&hex_pairs),
        fmt_map(&sgr_pairs)
    )
}

pub fn sync_syntax_artifacts(home: &str, pal: &Palette) {
    let pal_path = resolve_palette_path(home);
    let default_dir = Path::new(home).join(".config/cc-ide");
    let cfg_dir = pal_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(&default_dir);
    let digest = compute_digest(pal);
    let resolved_path = cfg_dir.join("syntax_resolved.json");
    let (tm_p, glow_p, gh_p, yz_p, lg_p) = (
        cfg_dir.join("ccide.tmTheme"),
        cfg_dir.join("glow-style.json"),
        cfg_dir.join("ghostty-theme.conf"),
        cfg_dir.join("yazi-theme.toml"),
        cfg_dir.join("lazygit.yml"),
    );
    if tm_p.is_file() && glow_p.is_file() && gh_p.is_file() && yz_p.is_file() && lg_p.is_file() {
        if let Ok(txt) = fs::read_to_string(&resolved_path) {
            if extract_json_str(&txt, "digest").as_deref() == Some(&digest) {
                return;
            }
        }
    }
    if fs::create_dir_all(cfg_dir).is_ok() {
        let tm_txt = build_tm_theme(pal);
        atomic_write(&tm_p, &tm_txt);
        atomic_write(&glow_p, &build_glow_style_json(pal));
        atomic_write(&gh_p, &build_ghostty_theme(pal));
        let bat_theme = sync_companion_themes(home, cfg_dir, pal, &tm_p, &tm_txt);
        atomic_write(
            &resolved_path,
            &build_syntax_resolved_json(pal, &digest, bat_theme),
        );
    }
}
