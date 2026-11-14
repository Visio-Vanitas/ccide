//! Palette-Unified Yazi (`theme.toml`), Lazygit (`lazygit.yml`) & `bat` Cache Sync (`<= 300` lines).

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::platform::find_executable;

use super::syntax::atomic_write;
use super::Palette;

pub(crate) fn build_yazi_theme_toml(pal: &Palette, tm_path: &Path) -> String {
    let (u, s) = (&pal.ui_specs, &pal.syntax);
    let [txt, dim, brd, cyn, blu, lav, amb, grn, cor, mnt, act_bg, cur_bg] = [
        u[0].to_hex_string(),
        u[1].to_hex_string(),
        u[2].to_hex_string(),
        u[3].to_hex_string(),
        u[4].to_hex_string(),
        u[5].to_hex_string(),
        u[6].to_hex_string(),
        u[7].to_hex_string(),
        u[8].to_hex_string(),
        u[9].to_hex_string(),
        u[10].to_hex_string(),
        u[11].to_hex_string(),
    ];
    let bg = s.code_bg.to_hex_string();
    let tm_s = tm_path.to_string_lossy().replace('\\', "/");
    format!(
        "# Managed by cc-ide: palette-unified Yazi UI & syntect theme (preset: {})\n\
         [mgr]\n\
         cwd             = {{ fg = \"{cyn}\" }}\n\
         hovered         = {{ fg = \"{txt}\", bg = \"{cur_bg}\", bold = true }}\n\
         preview_hovered = {{ fg = \"{txt}\", bg = \"{act_bg}\" }}\n\
         find_keyword    = {{ fg = \"{amb}\", bold = true, underline = true }}\n\
         find_position   = {{ fg = \"{lav}\", bg = \"reset\", bold = true }}\n\
         marker_copied   = {{ fg = \"{grn}\", bg = \"{grn}\" }}\n\
         marker_cut      = {{ fg = \"{cor}\", bg = \"{cor}\" }}\n\
         marker_selected = {{ fg = \"{amb}\", bg = \"{amb}\" }}\n\
         border_symbol   = \"│\"\n\
         border_style    = {{ fg = \"{brd}\" }}\n\
         syntect_theme   = \"{tm_s}\"\n\n\
         [tabs]\n\
         active   = {{ fg = \"{bg}\", bg = \"{cyn}\", bold = true }}\n\
         inactive = {{ fg = \"{txt}\", bg = \"{act_bg}\" }}\n\n\
         [mode]\n\
         normal_main = {{ fg = \"{bg}\", bg = \"{blu}\", bold = true }}\n\
         normal_alt  = {{ fg = \"{txt}\", bg = \"{act_bg}\" }}\n\
         select_main = {{ fg = \"{bg}\", bg = \"{lav}\", bold = true }}\n\
         select_alt  = {{ fg = \"{txt}\", bg = \"{act_bg}\" }}\n\
         unset_main  = {{ fg = \"{bg}\", bg = \"{cor}\", bold = true }}\n\
         unset_alt   = {{ fg = \"{txt}\", bg = \"{act_bg}\" }}\n\n\
         [status]\n\
         perm_sep        = {{ fg = \"{dim}\" }}\n\
         perm_type       = {{ fg = \"{blu}\" }}\n\
         perm_read       = {{ fg = \"{amb}\" }}\n\
         perm_write      = {{ fg = \"{cor}\" }}\n\
         perm_exec       = {{ fg = \"{grn}\" }}\n\
         progress_label  = {{ fg = \"{txt}\", bold = true }}\n\
         progress_normal = {{ fg = \"{cyn}\", bg = \"{act_bg}\" }}\n\
         progress_error  = {{ fg = \"{cor}\", bg = \"{act_bg}\" }}\n\n\
         [filetype]\n\
         rules = [\n  \
           {{ mime = \"image/*\", fg = \"{lav}\" }},\n  \
           {{ mime = \"{{audio,video}}/*\", fg = \"{amb}\" }},\n  \
           {{ mime = \"application/{{zip,rar,7z*,tar,gzip,xz,zstd,bzip*,lzma,compress,archive,cpio,arj,xar,ms-cab*}}\", fg = \"{cor}\" }},\n  \
           {{ mime = \"application/{{pdf,doc,rtf}}\", fg = \"{mnt}\" }},\n  \
           {{ url = \"*\", is = \"orphan\", fg = \"{cor}\" }},\n  \
           {{ url = \"*\", is = \"link\", fg = \"{cyn}\" }},\n  \
           {{ url = \"*\", is = \"exec\", fg = \"{grn}\" }},\n  \
           {{ url = \"*/\", fg = \"{blu}\", bold = true }},\n  \
           {{ url = \"*\", fg = \"{txt}\" }}\n\
         ]\n",
        pal.preset
    )
}

pub(crate) fn build_lazygit_yml(pal: &Palette) -> String {
    let (u, s) = (&pal.ui_specs, &pal.syntax);
    let [txt, brd, cyn, blu, amb, cor, act_bg, cur_bg, bg] = [
        u[0].to_hex_string(),
        u[2].to_hex_string(),
        u[3].to_hex_string(),
        u[4].to_hex_string(),
        u[6].to_hex_string(),
        u[8].to_hex_string(),
        u[10].to_hex_string(),
        u[11].to_hex_string(),
        s.code_bg.to_hex_string(),
    ];
    format!(
        "# Managed by cc-ide (preset: {}): full-width diff/log + palette-unified UI theme\n\
         gui:\n  \
           mouseEvents: true\n  \
           scrollHeight: 2\n  \
           scrollPastBottom: false\n  \
           wrapLinesInStagingView: false\n  \
           mainPanelSplitMode: vertical\n  \
           splitDiff: never\n  \
           commitHashLength: 0\n  \
           showBranchCommitHash: false\n  \
           sidePanelWidth: 0.32\n  \
           expandFocusedSidePanel: false\n  \
           statusPanelView: allBranchesLog\n  \
           theme:\n    \
             activeBorderColor:\n      - \"{cyn}\"\n      - bold\n    \
             inactiveBorderColor:\n      - \"{brd}\"\n    \
             searchingActiveBorderColor:\n      - \"{amb}\"\n      - bold\n    \
             optionsTextColor:\n      - \"{blu}\"\n    \
             selectedLineBgColor:\n      - \"{cur_bg}\"\n    \
             inactiveViewSelectedLineBgColor:\n      - default\n    \
             cherryPickedCommitFgColor:\n      - \"{cyn}\"\n    \
             cherryPickedCommitBgColor:\n      - \"{act_bg}\"\n    \
             markedBaseCommitFgColor:\n      - \"{bg}\"\n    \
             markedBaseCommitBgColor:\n      - \"{amb}\"\n    \
             unstagedChangesColor:\n      - \"{cor}\"\n    \
             defaultFgColor:\n      - \"{txt}\"\n\n\
         git:\n  \
           log:\n    order: topo-order\n    showGraph: always\n  \
           paging:\n    colorArg: always\n    pager: '\"$HOME/.claude/scripts/lazygit-pager.sh\" --col-width={{{{columnWidth}}}}'\n  \
           branchLogCmd: lazygit-log.py --branch={{{{branchName}}}}\n  \
           allBranchesLogCmds:\n    - lazygit-log.py --all\n\n\
         keybinding:\n  universal:\n    scrollLeft: H\n    scrollRight: L\n",
        pal.preset
    )
}

fn try_sync_bat_cache(home: &str, tm_content: &str) -> bool {
    let Some(bat) = find_executable("bat", home).or_else(|| find_executable("batcat", home)) else {
        return false;
    };
    let Ok(out) = Command::new(&bat)
        .arg("--config-dir")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let dir_s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if dir_s.is_empty() {
        return false;
    }
    let themes_dir = Path::new(&dir_s).join("themes");
    if fs::create_dir_all(&themes_dir).is_err() {
        return false;
    }
    atomic_write(&themes_dir.join("ccide.tmTheme"), tm_content);
    Command::new(&bat)
        .args(["cache", "--build"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) fn sync_companion_themes(
    home: &str,
    cfg_dir: &Path,
    pal: &Palette,
    tm_path: &Path,
    tm_content: &str,
) -> &'static str {
    let yazi_toml = build_yazi_theme_toml(pal, tm_path);
    atomic_write(&cfg_dir.join("yazi-theme.toml"), &yazi_toml);
    atomic_write(&cfg_dir.join("lazygit.yml"), &build_lazygit_yml(pal));

    let explicit_override = env::var("CCIDE_PALETTE_FILE")
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if explicit_override {
        return "base16-256";
    }

    let mut yazi_targets = vec![Path::new(home).join(".config/yazi/theme.toml")];
    if cfg!(windows) {
        if let Ok(appdata) = env::var("APPDATA") {
            if !appdata.trim().is_empty() {
                yazi_targets.push(Path::new(appdata.trim()).join("yazi/config/theme.toml"));
            }
        }
    }
    for yt in yazi_targets {
        let can_manage = match fs::read_to_string(&yt) {
            Err(_) => true,
            Ok(s) => s.trim().is_empty() || s.contains("Managed by cc-ide"),
        };
        if can_manage {
            if let Some(p) = yt.parent() {
                let _ = fs::create_dir_all(p);
            }
            atomic_write(&yt, &yazi_toml);
        }
    }

    if try_sync_bat_cache(home, tm_content) {
        "ccide"
    } else {
        "base16-256"
    }
}
