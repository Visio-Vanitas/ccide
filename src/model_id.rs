//! Leaf Model Identifier Validation & Agent Shim Slug Resolution (`src/model_id.rs`).
//!
//! Depends only on `std` so both `crate::session` and `crate::orchestrator` can share
//! model-name validation and shim resolution without circular module dependencies.

use std::fs;
use std::path::Path;

pub const GP_SHIM_PREFIX: &str = "ccide-gp--";
pub const RO_SHIM_PREFIX: &str = "ccide-ro--";

/// Rejects empty strings, angle-bracket placeholders (`<synthetic>`), and serialized
/// JS/JSON primitives (`false`, `true`, `null`, `undefined`, `none`, `nan`, `synthetic`).
pub fn is_valid_model_name(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() || s.starts_with('<') || s.ends_with('>') {
        return false;
    }
    !matches!(
        s.to_ascii_lowercase().as_str(),
        "false" | "true" | "null" | "undefined" | "none" | "nan" | "synthetic"
    )
}

pub fn model_slug(model_id: &str) -> String {
    let mut out = String::with_capacity(model_id.len());
    let mut prev_dash = false;
    for ch in model_id.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "inherit".to_string()
    } else {
        out
    }
}

pub fn gp_shim_name(model_id: &str) -> String {
    format!("{GP_SHIM_PREFIX}{}", model_slug(model_id))
}

pub fn ro_shim_name(model_id: &str) -> String {
    format!("{RO_SHIM_PREFIX}{}", model_slug(model_id))
}

/// Resolves `(clean_role_badge, model_id)` if `agent_type` is a managed `ccide-{gp,ro}--<slug>` shim.
pub fn model_from_shim_agent_type(home: &str, agent_type: &str) -> Option<(&'static str, String)> {
    let clean_name = agent_type.rsplit(':').next().unwrap_or(agent_type);
    if clean_name.contains('/') || clean_name.contains('\\') || clean_name.contains("..") {
        return None;
    }
    let role_badge = if clean_name.starts_with(RO_SHIM_PREFIX) {
        "scout"
    } else if clean_name.starts_with(GP_SHIM_PREFIX) {
        "worker"
    } else {
        return None;
    };
    let md_path = Path::new(home)
        .join(".claude")
        .join("agents")
        .join(format!("{}.md", clean_name));
    if let Ok(txt) = fs::read_to_string(md_path) {
        for line in txt.lines().take(12) {
            if let Some(rest) = line.trim().strip_prefix("model:") {
                let m = rest.trim().trim_matches('"').trim_matches('\'');
                if !m.is_empty() {
                    return Some((role_badge, m.to_string()));
                }
            }
        }
    }
    let slug = clean_name
        .strip_prefix(GP_SHIM_PREFIX)
        .or_else(|| clean_name.strip_prefix(RO_SHIM_PREFIX))?;
    Some((role_badge, slug.to_string()))
}
