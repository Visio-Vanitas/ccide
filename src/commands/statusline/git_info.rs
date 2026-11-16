//! Fast Zero-Subprocess Git HEAD / Linked Worktree Branch Resolver for `ccide-core statusline`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::tui::width::floor_char_boundary;

pub fn read_git_head_branch(dot_git: &Path) -> Option<String> {
    let head_path = if dot_git.is_dir() {
        dot_git.join("HEAD")
    } else if dot_git.is_file() {
        let ptr = fs::read_to_string(dot_git).ok()?;
        let gdir = ptr.trim().strip_prefix("gitdir:")?.trim();
        let resolved = if Path::new(gdir).is_absolute() {
            PathBuf::from(gdir)
        } else {
            dot_git.parent()?.join(gdir)
        };
        resolved.join("HEAD")
    } else {
        return None;
    };
    let htxt = fs::read_to_string(head_path).ok()?;
    let trimmed = htxt.trim();
    // Symbolic refs (`ref: refs/heads/main`, `ref: refs/tags/v1.0`, ...) resolve to a short name
    if let Some(sym) = trimmed.strip_prefix("ref:") {
        let rp = sym.trim();
        let br = rp
            .strip_prefix("refs/heads/")
            .or_else(|| rp.strip_prefix("refs/"))
            .unwrap_or(rp);
        return (!br.is_empty()).then(|| br.to_string());
    }
    let br = &trimmed[..floor_char_boundary(trimmed, trimmed.len().min(7))];
    (!br.is_empty()).then(|| br.to_string())
}

pub fn fast_git_summary(cwd: &str, wt_hint: &str) -> String {
    let mut cur = PathBuf::from(cwd);
    for _ in 0..64 {
        let dg = cur.join(".git");
        if let Some(branch) = read_git_head_branch(&dg) {
            if !wt_hint.is_empty() && wt_hint != branch {
                return format!("\u{e0a0} {}[{}]", branch, wt_hint);
            }
            return format!("\u{e0a0} {}", branch);
        }
        if !cur.pop() {
            break;
        }
    }
    if !wt_hint.is_empty() {
        return format!("\u{e0a0} {}", wt_hint);
    }
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("~")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_worktree_hint_or_basename() {
        assert_eq!(
            fast_git_summary("/nonexistent/my-repo", "feat-wt"),
            "\u{e0a0} feat-wt"
        );
        assert_eq!(fast_git_summary("/nonexistent/my-repo", ""), "my-repo");
    }
}
