//! Setup Options Persistence & Streaming `install.sh` Subprocess Runner.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::json::{extract_json_bool, extract_json_str};
use crate::tui::palette::{render_palette_template, resolve_palette_path, Palette};

pub const PRESETS: [&str; 6] = [
    "default",
    "catppuccin-mocha",
    "tokyo-night",
    "nord",
    "gruvbox-dark",
    "dracula",
];

pub const SPRITE_MODES: [&str; 3] = ["auto", "ascii", "image"];

#[derive(Clone, Debug)]
pub struct SetupOptions {
    pub preset_idx: usize,
    pub sprite_mode_idx: usize,
    pub wrap_lines: bool,
    pub install_plugin: bool,
    pub patch_claude: bool,
    pub config_companions: bool,
    pub setup_daemon_ssh: bool,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            preset_idx: 0,
            sprite_mode_idx: 0,
            wrap_lines: false,
            install_plugin: true,
            patch_claude: true,
            config_companions: true,
            setup_daemon_ssh: true,
        }
    }
}

pub fn load_initial_options(home: &str) -> SetupOptions {
    let mut opts = SetupOptions::default();
    let pal = Palette::load(home);
    if let Some(pos) = PRESETS.iter().position(|&p| p == pal.preset) {
        opts.preset_idx = pos;
    }
    if let Ok(raw) = fs::read_to_string(Path::new(home).join(".config/cc-ide/viewer.json")) {
        if let Some(w) = extract_json_bool(&raw, "wrap") {
            opts.wrap_lines = w;
        }
        if let Some(sm) = extract_json_str(&raw, "sprite_mode") {
            if let Some(pos) = SPRITE_MODES
                .iter()
                .position(|&m| m.eq_ignore_ascii_case(sm.trim()))
            {
                opts.sprite_mode_idx = pos;
            }
        }
    }
    opts
}

pub fn write_user_preferences(home: &str, opts: &SetupOptions) {
    let cfg_dir = Path::new(home).join(".config/cc-ide");
    let _ = fs::create_dir_all(&cfg_dir);

    let chosen_preset = PRESETS[opts.preset_idx % PRESETS.len()];
    let pal_path = resolve_palette_path(home);
    let existing_preset = fs::read_to_string(&pal_path)
        .ok()
        .map(|s| Palette::from_json_str(&s).preset);
    if existing_preset.as_deref() != Some(chosen_preset) || !pal_path.exists() {
        let _ = fs::write(&pal_path, render_palette_template(chosen_preset));
    }

    let chosen_sprite = SPRITE_MODES[opts.sprite_mode_idx % SPRITE_MODES.len()];
    let viewer_json = format!(
        "{{\n  // Line-wrapping & sprite rendering mode (\"auto\" uses PNG/JPG on iTerm2/Kitty/Ghostty, .ansi elsewhere)\n  \"wrap\": {},\n  \"wrap_style\": \"vi\",\n  \"sprite_mode\": \"{}\"\n}}\n",
        if opts.wrap_lines { "true" } else { "false" },
        chosen_sprite
    );
    let _ = fs::write(cfg_dir.join("viewer.json"), viewer_json);
}

fn strip_ansi_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn step_row_value(opts: &mut SetupOptions, row_idx: usize, forward: bool) -> bool {
    match row_idx {
        0 => {
            opts.preset_idx = if forward {
                (opts.preset_idx + 1) % PRESETS.len()
            } else {
                (opts.preset_idx + PRESETS.len() - 1) % PRESETS.len()
            };
            true
        }
        1 => {
            opts.sprite_mode_idx = if forward {
                (opts.sprite_mode_idx + 1) % SPRITE_MODES.len()
            } else {
                (opts.sprite_mode_idx + SPRITE_MODES.len() - 1) % SPRITE_MODES.len()
            };
            false
        }
        2 => {
            opts.wrap_lines = !opts.wrap_lines;
            false
        }
        3 => {
            opts.install_plugin = !opts.install_plugin;
            false
        }
        4 => {
            opts.patch_claude = !opts.patch_claude;
            false
        }
        5 => {
            opts.config_companions = !opts.config_companions;
            false
        }
        6 => {
            opts.setup_daemon_ssh = !opts.setup_daemon_ssh;
            false
        }
        _ => false,
    }
}

pub fn execute_install_pipeline(
    home: &str,
    project_root: &Path,
    opts: &SetupOptions,
    uninstall: bool,
    on_log: &mut dyn FnMut(String),
) -> bool {
    if !uninstall {
        write_user_preferences(home, opts);
        on_log(format!(
            "[OK] Saved palette '{}', sprite_mode '{}' & wrap={}",
            PRESETS[opts.preset_idx % PRESETS.len()],
            SPRITE_MODES[opts.sprite_mode_idx % SPRITE_MODES.len()],
            opts.wrap_lines
        ));
    }

    let install_sh = project_root.join("install.sh");
    if !install_sh.is_file() {
        on_log(format!(
            "[ERR] install.sh not found under {}",
            project_root.display()
        ));
        return false;
    }

    let script_cmd = if uninstall {
        format!("exec zsh {:?} --uninstall 2>&1", install_sh)
    } else {
        format!("exec zsh {:?} --apply 2>&1", install_sh)
    };

    let mut child = match Command::new("zsh")
        .args(["-c", &script_cmd])
        .env("CCIDE_NONINTERACTIVE", "1")
        .env("CCIDE_SKIP_BUILD", "1")
        .env(
            "CCIDE_OPT_PLUGIN",
            if opts.install_plugin { "1" } else { "0" },
        )
        .env("CCIDE_OPT_PATCH", if opts.patch_claude { "1" } else { "0" })
        .env(
            "CCIDE_OPT_COMPANION",
            if opts.config_companions { "1" } else { "0" },
        )
        .env(
            "CCIDE_OPT_DAEMON",
            if opts.setup_daemon_ssh { "1" } else { "0" },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            on_log(format!("[ERR] Failed to spawn install.sh: {e}"));
            return false;
        }
    };

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let clean = strip_ansi_simple(&line);
            if !clean.trim().is_empty() {
                on_log(clean);
            }
        }
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}
