//! `ccide-core setup` & `ccide-core doctor`: Interactive TUI Installer & Diagnostics.

pub mod doctor;
pub mod doctor_render;
pub mod input;
pub mod runner;
pub mod ui;

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::Ordering;

use crate::ffi::{install_signal_handlers, RUNNING};
use crate::platform::user_home_dir;
use crate::tui::grid::{get_terminal_geometry, render_multi_panel_grid};
use crate::tui::palette::{render_palette_template, Palette};

pub use doctor::run_doctor;
use doctor::{collect_doctor_report, resolve_project_root};
use input::{
    parse_setup_keys, RawTermGuard, SetupKey, TAG_APPLY_BTN, TAG_SETUP_ROW, TOTAL_SETUP_ROWS,
};
use runner::{execute_install_pipeline, load_initial_options, step_row_value, PRESETS};
use ui::build_setup_panels;

pub fn run_setup(args: &[String]) {
    let home = user_home_dir();
    let project_root = resolve_project_root(args.get(2).map(|s| s.as_str()), &home);

    install_signal_handlers(false);

    let guard = RawTermGuard::enter();
    let mut stdout = std::io::stdout();

    let mut report = collect_doctor_report(&home, &project_root);
    let mut opts = load_initial_options(&home);
    let mut cursor = TOTAL_SETUP_ROWS - 1;
    let mut frame_buf = String::with_capacity(16384);
    let mut prev_rows: Vec<String> = Vec::new();
    let mut row_tag_map: HashMap<usize, (u8, usize)> = HashMap::new();
    let mut prev_geom = (0usize, 0usize);
    let mut logs: Vec<String> = Vec::new();
    let mut finished_msg: Option<String> = None;

    while RUNNING.load(Ordering::Relaxed) {
        let geom = get_terminal_geometry();
        if (geom.rows, geom.cols) != prev_geom {
            prev_geom = (geom.rows, geom.cols);
            prev_rows.clear();
        }

        let live_pal = Palette::from_json_str(&render_palette_template(
            PRESETS[opts.preset_idx % PRESETS.len()],
        ));
        let panels = build_setup_panels(
            geom.cols,
            geom.rows,
            &report,
            &opts,
            cursor,
            &logs,
            finished_msg.as_deref(),
            &live_pal,
        );
        render_multi_panel_grid(
            geom,
            &panels,
            &live_pal,
            &mut frame_buf,
            &mut row_tag_map,
            &mut prev_rows,
        );
        if !frame_buf.is_empty() {
            let _ = stdout.write_all(frame_buf.as_bytes());
            let _ = stdout.flush();
        }

        let mut in_buf = [0u8; 256];
        let Some(n) = guard.session.poll_stdin(&mut in_buf, 500) else {
            continue;
        };
        let mut run_action: Option<bool> = None;

        for key in parse_setup_keys(&String::from_utf8_lossy(&in_buf[..n])) {
            if finished_msg.is_some() {
                if matches!(
                    key,
                    SetupKey::Enter
                        | SetupKey::Quit
                        | SetupKey::ToggleOrActivate
                        | SetupKey::Click { .. }
                ) {
                    return;
                }
                continue;
            }
            match key {
                SetupKey::Quit => return,
                SetupKey::Up => {
                    cursor = if cursor == 0 {
                        TOTAL_SETUP_ROWS - 1
                    } else {
                        cursor - 1
                    };
                }
                SetupKey::Down => cursor = (cursor + 1) % TOTAL_SETUP_ROWS,
                SetupKey::PresetIdx(idx) if idx < PRESETS.len() => {
                    opts.preset_idx = idx;
                    prev_rows.clear();
                }
                SetupKey::PresetIdx(_) => {}
                SetupKey::RefreshDoctor => {
                    report = collect_doctor_report(&home, &project_root);
                    prev_rows.clear();
                }
                SetupKey::Left => {
                    if step_row_value(&mut opts, cursor, false) {
                        prev_rows.clear();
                    }
                }
                SetupKey::Right | SetupKey::ToggleOrActivate => {
                    if cursor == TOTAL_SETUP_ROWS - 1 {
                        run_action = Some(false);
                    } else if step_row_value(&mut opts, cursor, true) {
                        prev_rows.clear();
                    }
                }
                SetupKey::Enter => {
                    if cursor == TOTAL_SETUP_ROWS - 1 {
                        run_action = Some(false);
                    } else if step_row_value(&mut opts, cursor, true) {
                        prev_rows.clear();
                    }
                }
                SetupKey::InstallNow => run_action = Some(false),
                SetupKey::Uninstall => run_action = Some(true),
                SetupKey::Click { col: _, row } => {
                    if let Some(&(tkind, tidx)) = row_tag_map.get(&row) {
                        if tkind == TAG_APPLY_BTN {
                            cursor = TOTAL_SETUP_ROWS - 1;
                            run_action = Some(false);
                        } else if tkind == TAG_SETUP_ROW {
                            cursor = tidx;
                            if step_row_value(&mut opts, tidx, true) {
                                prev_rows.clear();
                            }
                        }
                    }
                }
            }
        }

        if let Some(is_uninstall) = run_action {
            logs.clear();
            prev_rows.clear();
            logs.push(if is_uninstall {
                "Starting clean cc-ide uninstallation & backup restoration...".to_string()
            } else {
                "Starting cc-ide modular installation & workspace hot-reload...".to_string()
            });
            let ok =
                execute_install_pipeline(&home, &project_root, &opts, is_uninstall, &mut |line| {
                    logs.push(line);
                    let g = get_terminal_geometry();
                    let p = build_setup_panels(
                        g.cols, g.rows, &report, &opts, cursor, &logs, None, &live_pal,
                    );
                    render_multi_panel_grid(
                        g,
                        &p,
                        &live_pal,
                        &mut frame_buf,
                        &mut row_tag_map,
                        &mut prev_rows,
                    );
                    if !frame_buf.is_empty() {
                        let _ = stdout.write_all(frame_buf.as_bytes());
                        let _ = stdout.flush();
                    }
                });
            report = collect_doctor_report(&home, &project_root);
            finished_msg = Some(if ok {
                if is_uninstall {
                    "[✓] Clean uninstall complete! Press Enter or q to exit.".to_string()
                } else {
                    "[✓] cc-ide installed & hot-reloaded! Press Enter or q to exit.".to_string()
                }
            } else {
                "[!] Completed with errors (inspect log above). Press Enter or q to exit."
                    .to_string()
            });
            prev_rows.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::setup::doctor::DoctorItem;
    use crate::commands::setup::runner::SetupOptions;

    #[test]
    fn parses_setup_keys_and_renders_80x24() {
        let keys = parse_setup_keys("\x1b[A\x1b[B 3\x1b[<0;18;12M\r");
        assert_eq!(keys.len(), 6);
        let pal = Palette::default_palette();
        let rep = doctor::DoctorReport {
            os_arch: "darwin/aarch64".to_string(),
            backend_tier: "Tier 1 · Local iTerm2".to_string(),
            items: vec![DoctorItem {
                status: doctor::CheckStatus::Ok,
                label: "Claude Code".to_string(),
                detail: "verified".to_string(),
            }],
            missing_hint: "OK".to_string(),
        };
        let panels = build_setup_panels(80, 24, &rep, &SetupOptions::default(), 8, &[], None, &pal);
        assert_eq!(panels.len(), 3);
    }
}
