//! Multi-Agent Host Identity (`Claude` / `Agy` / `Codex`) & Tab Persistence (`src/session/agent.rs`).

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::platform::{atomic_write, cache_dir, user_home_dir};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentKind {
    #[default]
    Claude,
    Agy,
    Codex,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Agy => "agy",
            AgentKind::Codex => "codex",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Some(AgentKind::Claude),
            "agy" | "antigravity" | "antigravity-cli" => Some(AgentKind::Agy),
            "codex" | "openai-codex" | "codex-cli" => Some(AgentKind::Codex),
            _ => None,
        }
    }

    pub fn default_family(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Agy => "gemini",
            AgentKind::Codex => "openai",
        }
    }

    pub fn supports_view_switch(self) -> bool {
        matches!(self, AgentKind::Claude)
    }

    pub fn badge(self) -> &'static str {
        match self {
            AgentKind::Claude => "main",
            AgentKind::Agy => "agy",
            AgentKind::Codex => "codex",
        }
    }
}

/// Rejects empty IDs and path-traversal characters (`/`, `\`, `..`, NUL) before joining into IPC/cache filenames.
pub fn is_safe_id(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains('\\') && !s.contains("..") && !s.contains('\0')
}

pub fn read_tab_agent(home: &str, yid: &str) -> AgentKind {
    if !is_safe_id(yid) {
        return AgentKind::Claude;
    }
    let path = cache_dir(home).join(format!("tab_agent_{}.txt", yid));
    fs::read_to_string(path)
        .ok()
        .and_then(|s| AgentKind::from_name(&s))
        .unwrap_or(AgentKind::Claude)
}

pub fn write_tab_agent(home: &str, yid: &str, k: AgentKind) {
    if !is_safe_id(yid) {
        return;
    }
    let cdir = cache_dir(home);
    let _ = fs::create_dir_all(&cdir);
    let path = cdir.join(format!("tab_agent_{}.txt", yid));
    let body = format!("{}\n", k.as_str());
    let _ = atomic_write(&path, body.as_bytes());
}

fn env_dir_or(var_name: &str, default_tail: &[&str]) -> PathBuf {
    if let Ok(val) = env::var(var_name) {
        let t = val.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    let mut p = PathBuf::from(user_home_dir());
    for seg in default_tail {
        p.push(seg);
    }
    p
}

pub fn agy_home() -> PathBuf {
    env_dir_or("CCIDE_AGY_HOME", &[".gemini", "antigravity-cli"])
}

pub fn codex_home() -> PathBuf {
    env_dir_or("CODEX_HOME", &[".codex"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_kind_roundtrip_and_tab_persistence() {
        assert_eq!(AgentKind::default(), AgentKind::Claude);
        assert_eq!(AgentKind::from_name("claude-code"), Some(AgentKind::Claude));
        assert_eq!(AgentKind::from_name("antigravity"), Some(AgentKind::Agy));
        assert_eq!(AgentKind::from_name("openai-codex"), Some(AgentKind::Codex));
        assert_eq!(AgentKind::from_name("unknown"), None);
        assert_eq!(AgentKind::Agy.default_family(), "gemini");
        assert_eq!(AgentKind::Codex.default_family(), "openai");
        assert!(AgentKind::Claude.supports_view_switch());
        assert!(!AgentKind::Agy.supports_view_switch());
        assert!(!AgentKind::Codex.supports_view_switch());
        assert_eq!(AgentKind::Claude.badge(), "main");
        assert_eq!(AgentKind::Agy.badge(), "agy");
        assert_eq!(AgentKind::Codex.badge(), "codex");
        assert!(is_safe_id("abc-123_ok"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("../x"));
        assert!(!is_safe_id("a/b"));
        assert!(!is_safe_id(r"a\b"));

        let tmp = env::temp_dir().join(format!("ccide_agent_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let home = tmp.to_string_lossy().into_owned();
        assert_eq!(read_tab_agent(&home, "y99"), AgentKind::Claude);
        write_tab_agent(&home, "y99", AgentKind::Codex);
        assert_eq!(read_tab_agent(&home, "y99"), AgentKind::Codex);
        write_tab_agent(&home, "y99", AgentKind::Agy);
        assert_eq!(read_tab_agent(&home, "y99"), AgentKind::Agy);
        write_tab_agent(&home, "../evil", AgentKind::Codex);
        assert_eq!(read_tab_agent(&home, "../evil"), AgentKind::Claude);
        let _ = fs::remove_dir_all(&tmp);
    }
}
