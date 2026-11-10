//! Resolution-Adaptive 2x2 Quadrant Sub-Pixel Vector Logo Loader & Panel Builder.

use std::collections::HashMap;
use std::fs;

use crate::tui::RESET;
use crate::tui::grid::{BoxRow, PanelSpec, TermGeom};
use crate::tui::width::display_width;

/// Loads pre-rendered resolution-adaptive 2x2 Quadrant logos (`adaptive_quadrant_logos.json`)
/// for `gemini`, `deepseek`, `claude`, `openai`, and `grok`.
pub fn load_adaptive_logos(home: &str) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    let path = format!("{}/.cache/cc-ide/logos/adaptive_quadrant_logos.json", home);
    if let Ok(txt) = fs::read_to_string(&path) {
        for fam in ["gemini", "deepseek", "claude", "openai", "grok"] {
            let key = format!("\"{}\":", fam);
            if let Some(pos) = txt.find(&key) {
                let rest = &txt[pos + key.len()..];
                if let Some(arr_start) = rest.find('[')
                    && let Some(arr_end) = rest[arr_start..].find(']')
                {
                    let arr_body = &rest[arr_start + 1..arr_start + arr_end];
                    let mut lines = Vec::new();
                    let mut cur = String::new();
                    let mut in_str = false;
                    let mut chars = arr_body.chars();
                    while let Some(c) = chars.next() {
                        if !in_str {
                            if c == '"' {
                                in_str = true;
                                cur.clear();
                            }
                        } else if c == '\\' {
                            if let Some(esc) = chars.next() {
                                match esc {
                                    'u' => {
                                        let hex: String = chars.by_ref().take(4).collect();
                                        if let Ok(code) = u32::from_str_radix(&hex, 16)
                                            && let Some(ch) = char::from_u32(code)
                                        {
                                            cur.push(ch);
                                        }
                                    }
                                    'n' => cur.push('\n'),
                                    '"' => cur.push('"'),
                                    '\\' => cur.push('\\'),
                                    other => cur.push(other),
                                }
                            }
                        } else if c == '"' {
                            in_str = false;
                            lines.push(cur.clone());
                        } else {
                            cur.push(c);
                        }
                    }
                    if !lines.is_empty() {
                        map.insert(fam.to_string(), lines);
                    }
                }
            }
        }
    }
    map
}

/// Builds the Top Logo `PanelSpec` for the active model family (`gemini`, `deepseek`, `claude`, etc.).
pub fn build_top_logo_panel(
    geom: TermGeom,
    logos: &HashMap<String, Vec<String>>,
    family: &str,
    accent: &'static str,
    badge: String,
) -> (PanelSpec, usize) {
    let mut logo_rows = Vec::new();
    logo_rows.push(BoxRow {
        text: String::new(),
        bg: "",
        tag: None,
    });
    if let Some(art) = logos.get(family) {
        let vis_w = art.first().map(|l| display_width(l)).unwrap_or(26);
        let pad_left = geom.cols.saturating_sub(2).saturating_sub(vis_w) / 2;
        let prefix = " ".repeat(pad_left);
        for line in art {
            logo_rows.push(BoxRow {
                text: format!("{}{}{RESET}", prefix, line),
                bg: "",
                tag: None,
            });
        }
    } else {
        logo_rows.push(BoxRow {
            text: format!("  {accent}{}{RESET}", family.to_uppercase()),
            bg: "",
            tag: None,
        });
    }
    logo_rows.push(BoxRow {
        text: String::new(),
        bg: "",
        tag: None,
    });
    let top_h = logo_rows.len() + 2;
    let panel = PanelSpec {
        title: family.to_uppercase(),
        badge,
        accent,
        rows: logo_rows,
        fixed_height: Some(top_h),
        weight: 1.0,
        raw_interior: false,
    };
    (panel, top_h)
}
