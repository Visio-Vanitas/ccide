//! Versioned `PaneSpec` Wire Format (`v2` + Legacy `v1`) (`src/commands/workspace/spec.rs`).

pub use crate::commands::workspace::spec_io::{
    prune_dead_specs, read_pane_spec, remove_tab_state, write_pane_spec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Iterm,
    Osc,
    Tmux,
    WezTerm,
    Wt,
    Ghostty,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Iterm => "iterm",
            Self::Osc => "osc",
            Self::Tmux => "tmux",
            Self::WezTerm => "wezterm",
            Self::Wt => "wt",
            Self::Ghostty => "ghostty",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "iterm" | "iterm2" => Some(Self::Iterm),
            "osc" | "ssh-osc" => Some(Self::Osc),
            "tmux" => Some(Self::Tmux),
            "wezterm" => Some(Self::WezTerm),
            "wt" => Some(Self::Wt),
            "ghostty" => Some(Self::Ghostty),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSpec {
    pub backend: Backend,
    pub yid: String,
    pub center: String,
    pub yazi: Option<String>,
    pub lazygit: Option<String>,
    pub mon: Option<String>,
    pub sidebar: Option<String>,
    pub left_pct: u8,
    pub show_git: bool,
    pub show_mon: bool,
    pub show_right: bool,
    pub legacy_sides: Vec<String>,
    pub legacy: bool,
}

pub struct OpenRequest<'a> {
    pub home: &'a str,
    pub target_dir: &'a str,
    pub yazi_id: &'a str,
    pub git_root: &'a str,
    pub show_git: bool,
    pub show_mon: bool,
    pub show_right: bool,
    pub left_pct: u8,
    pub shell_pid: u32,
}

fn is_valid_v2_val(v: &str) -> bool {
    !v.is_empty()
        && v.bytes().all(|b| {
            b.is_ascii()
                && !b.is_ascii_control()
                && !b.is_ascii_whitespace()
                && !matches!(b, b';' | b'=' | b',')
        })
}

fn valid_opt(v: &str) -> Option<String> {
    is_valid_v2_val(v).then(|| v.to_string())
}

impl PaneSpec {
    fn base(backend: Backend, yid: String, legacy: bool) -> Self {
        Self {
            backend,
            yid,
            center: String::new(),
            yazi: None,
            lazygit: None,
            mon: None,
            sidebar: None,
            left_pct: 30,
            show_git: false,
            show_mon: false,
            show_right: false,
            legacy_sides: Vec::new(),
            legacy,
        }
    }

    pub fn parse(raw: &str, yid_hint: &str) -> Option<Self> {
        let s = raw.trim();
        if s.is_empty() {
            return None;
        }
        if s == "v2" || s.starts_with("v2;") {
            let mut spec = Self::base(Backend::Tmux, String::new(), false);
            let mut has_b = false;
            for part in s.split(';').skip(1).filter(|p| !p.is_empty()) {
                let (k, v) = part.split_once('=')?;
                match k {
                    "b" => {
                        spec.backend = Backend::from_name(v)?;
                        has_b = true;
                    }
                    "k" => spec.yid = valid_opt(v)?,
                    "c" if !v.is_empty() => spec.center = valid_opt(v)?,
                    "y" if !v.is_empty() => spec.yazi = Some(valid_opt(v)?),
                    "l" if !v.is_empty() => spec.lazygit = Some(valid_opt(v)?),
                    "m" if !v.is_empty() => spec.mon = Some(valid_opt(v)?),
                    "s" if !v.is_empty() => spec.sidebar = Some(valid_opt(v)?),
                    "lp" => spec.left_pct = v.parse::<u8>().ok()?,
                    "f" => {
                        spec.show_git = v.contains('g');
                        spec.show_mon = v.contains('m');
                        spec.show_right = v.contains('r');
                    }
                    _ => {}
                }
            }
            if !has_b {
                return None;
            }
            if spec.yid.is_empty() {
                spec.yid = yid_hint.trim().to_string();
            }
            return (!spec.yid.is_empty()).then_some(spec);
        }
        Self::parse_v1(s, yid_hint.trim())
    }

    fn parse_v1(s: &str, yid_hint: &str) -> Option<Self> {
        if s.contains(';')
            || s.contains('=')
            || s.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return None;
        }
        let mk_v1 = |backend, yid: &str, center: &str, csv: &str| -> PaneSpec {
            let mut it = csv.split(',').map(str::trim).filter(|x| !x.is_empty());
            let mut sp = Self::base(backend, yid.to_string(), true);
            sp.center = center.to_string();
            sp.yazi = it.next().map(String::from);
            sp.legacy_sides = it.map(String::from).collect();
            sp.show_right = !sp.legacy_sides.is_empty();
            sp
        };
        if let Some((prefix, rest)) = s.split_once(':') {
            if let Some(backend) = Backend::from_name(prefix) {
                return match backend {
                    Backend::Tmux | Backend::WezTerm | Backend::Ghostty => {
                        let (c, csv) = rest.split_once(':').unwrap_or((rest, ""));
                        (!c.trim().is_empty()).then(|| mk_v1(backend, yid_hint, c.trim(), csv))
                    }
                    Backend::Wt | Backend::Osc => {
                        let yid = if !rest.trim().is_empty() {
                            rest.trim()
                        } else {
                            yid_hint
                        };
                        (!yid.is_empty()).then(|| Self::base(backend, yid.to_string(), true))
                    }
                    Backend::Iterm => {
                        let sp = mk_v1(Backend::Iterm, yid_hint, "", rest);
                        sp.yazi.is_some().then_some(sp)
                    }
                };
            }
        }
        let sp = mk_v1(Backend::Iterm, yid_hint, "", s);
        sp.yazi.is_some().then_some(sp)
    }

    pub fn encode_v2(&self) -> Option<String> {
        if !self.legacy_sides.is_empty() || !is_valid_v2_val(&self.yid) {
            return None;
        }
        if !self.center.is_empty() && !is_valid_v2_val(&self.center) {
            return None;
        }
        for v in [&self.yazi, &self.lazygit, &self.mon, &self.sidebar]
            .into_iter()
            .flatten()
        {
            if !is_valid_v2_val(v) {
                return None;
            }
        }
        let mut out = format!("v2;b={};k={}", self.backend.as_str(), self.yid);
        if !self.center.is_empty() {
            out.push_str(&format!(";c={}", self.center));
        }
        for (k, opt) in [
            ("y", &self.yazi),
            ("l", &self.lazygit),
            ("m", &self.mon),
            ("s", &self.sidebar),
        ] {
            if let Some(v) = opt {
                out.push_str(&format!(";{k}={v}"));
            }
        }
        let mut f = String::with_capacity(3);
        if self.show_git {
            f.push('g');
        }
        if self.show_mon {
            f.push('m');
        }
        if self.show_right {
            f.push('r');
        }
        out.push_str(&format!(";lp={};f={f}", self.left_pct));
        Some(out)
    }

    pub fn encode(&self) -> String {
        if let Some(v2) = self.encode_v2() {
            return v2;
        }
        match self.backend {
            Backend::Wt | Backend::Osc => format!("{}:{}", self.backend.as_str(), self.yid),
            Backend::Iterm => format!("iterm:{}", self.all_companion_ids().join(",")),
            Backend::Tmux | Backend::WezTerm | Backend::Ghostty => {
                format!(
                    "{}:{}:{}",
                    self.backend.as_str(),
                    self.center,
                    self.all_companion_ids().join(",")
                )
            }
        }
    }

    pub fn all_companion_ids(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in [
            self.yazi.as_deref(),
            self.lazygit.as_deref(),
            self.mon.as_deref(),
            self.sidebar.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(self.legacy_sides.iter().map(String::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != self.center)
        {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_v2_and_parses_legacy_v1_specs() {
        let raw = "v2;b=tmux;k=1234;c=%1;y=%2;l=%3;s=%4;lp=30;f=gr";
        let sp = PaneSpec::parse(raw, "hint").expect("v2");
        assert_eq!(sp.encode_v2().as_deref(), Some(raw));
        assert_eq!(sp.all_companion_ids(), vec!["%2", "%3", "%4"]);
        assert!(sp.show_git && !sp.show_mon && sp.show_right && !sp.legacy);

        let v1_tm = PaneSpec::parse("tmux:%10:%11,%12,%13", "99").expect("v1 tmux");
        assert!(v1_tm.legacy && v1_tm.show_right && v1_tm.yazi.as_deref() == Some("%11"));
        assert_eq!(v1_tm.all_companion_ids(), vec!["%11", "%12", "%13"]);
        assert_eq!(v1_tm.encode(), "tmux:%10:%11,%12,%13");

        let v1_it = PaneSpec::parse("w0t0p1:GUID1,w0t0p2:GUID2", "77").expect("v1 iterm");
        assert_eq!(v1_it.backend, Backend::Iterm);
        assert_eq!(
            v1_it.all_companion_ids(),
            vec!["w0t0p1:GUID1", "w0t0p2:GUID2"]
        );
    }
}
