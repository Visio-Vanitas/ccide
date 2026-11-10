//! Multi-Row (`8 × height` Sub-Pixel) Oscilloscope Curve Renderer & Telemetry Panel Builder.

use crate::telemetry::TelemetrySampler;
use crate::tui::grid::BoxRow;
use crate::tui::{
    AMBER, BLUE, BOLD, CORAL, CYAN, DIM, GREEN, GRID_DOT, LAVENDER, MINT, RESET, SPARK_BLOCKS,
    WHITE,
};

pub fn fmt_rate(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 {
        format!("{:.1}M", bps / (1024.0 * 1024.0))
    } else if bps >= 1024.0 {
        format!("{:.0}K", bps / 1024.0)
    } else {
        format!("{:.0}B", bps)
    }
}

/// Renders a `height`-row × `width`-col sub-pixel block curve (`▁▂▃▄▅▆▇█`)
/// with a faint coordinate reticle (`·`) behind empty cells.
pub fn render_multirow_curve(
    samples: &[f64],
    width: usize,
    height: usize,
    col_low: &'static str,
    col_mid: &'static str,
    col_high: &'static str,
) -> Vec<String> {
    let height = height.max(1);
    let width = width.max(8);
    let start = samples.len().saturating_sub(width);
    let sliced = &samples[start..];
    let pad_n = width.saturating_sub(sliced.len());
    let first_v = sliced.first().copied().unwrap_or(0.0);

    let total_eighths = (height * 8) as f64;
    let mut rows_out = Vec::with_capacity(height);

    for r in 0..height {
        let row_from_bottom = (height - 1 - r) as i32;
        let row_color = if r == 0 && height >= 3 {
            col_high
        } else if r < height - 1 {
            col_mid
        } else {
            col_low
        };
        let mut s = String::with_capacity(width * 8);
        let mut active_color = "";

        for x in 0..width {
            let val = if x < pad_n {
                first_v
            } else {
                sliced[x - pad_n]
            };
            let v = val.clamp(0.0, 100.0);
            let mut fill_8 = ((v / 100.0) * total_eighths).round() as i32;
            if v > 0.2 && fill_8 == 0 {
                fill_8 = 1;
            }
            let rem = fill_8 - row_from_bottom * 8;

            if rem <= 0 {
                if !active_color.is_empty() {
                    s.push_str(RESET);
                    active_color = "";
                }
                if x % 6 == 0 {
                    s.push_str(GRID_DOT);
                } else {
                    s.push(' ');
                }
            } else {
                let lvl = (rem.min(8)) as usize;
                let ch = SPARK_BLOCKS[lvl];
                let c_use = if v >= 80.0 {
                    col_high
                } else if v >= 45.0 {
                    col_mid
                } else {
                    row_color
                };
                if c_use != active_color {
                    s.push_str(c_use);
                    active_color = c_use;
                }
                s.push(ch);
            }
        }
        if !active_color.is_empty() {
            s.push_str(RESET);
        }
        rows_out.push(s);
    }
    rows_out
}

/// Builds exact-height `BoxRow`s for the bottom `M4 PRO` Telemetry Panel (`CPU`, `GPU`, `MEM`, `NET`).
pub fn build_telemetry_panel_rows(
    sampler: &TelemetrySampler,
    inner_w: usize,
    panel_height: usize,
) -> Vec<BoxRow> {
    let avail = panel_height.saturating_sub(2).max(8);
    let curve_w = inner_w.saturating_sub(2).max(10);
    let use_dividers = avail >= 15;
    let overhead = if use_dividers { 7 } else { 4 };
    let curve_budget = avail.saturating_sub(overhead).max(4);

    let h_cpu = (curve_budget / 4 + usize::from(curve_budget % 4 >= 1)).max(1);
    let h_gpu = (curve_budget / 4 + usize::from(curve_budget % 4 >= 2)).max(1);
    let h_mem = (curve_budget / 4 + usize::from(curve_budget % 4 >= 3)).max(1);
    let h_net = curve_budget.saturating_sub(h_cpu + h_gpu + h_mem).max(1);

    let div_str = format!(" \x1b[38;2;48;52;70m{}{RESET}", "┄".repeat(curve_w));
    let mut rows = Vec::with_capacity(avail);

    // 1. CPU
    let cpu_c = if sampler.cpu_pct >= 80.0 {
        CORAL
    } else if sampler.cpu_pct >= 45.0 {
        AMBER
    } else {
        MINT
    };
    rows.push(BoxRow {
        text: format!(
            " {cpu_c}●{RESET} {BOLD}{WHITE}CPU{RESET} {DIM}{}C{RESET}  {BOLD}{cpu_c}{:4.1}%{RESET}  {DIM}load {:.2}{RESET}",
            sampler.cpu_cores, sampler.cpu_pct, sampler.load_1m
        ),
        bg: "",
        tag: None,
    });
    for cl in render_multirow_curve(&sampler.cpu_hist, curve_w, h_cpu, MINT, CYAN, CORAL) {
        rows.push(BoxRow {
            text: format!(" {}", cl),
            bg: "",
            tag: None,
        });
    }
    if use_dividers {
        rows.push(BoxRow {
            text: div_str.clone(),
            bg: "",
            tag: None,
        });
    }

    // 2. GPU
    let gpu_c = if sampler.gpu_pct >= 80.0 {
        CORAL
    } else if sampler.gpu_pct >= 45.0 {
        AMBER
    } else {
        BLUE
    };
    #[cfg(target_os = "macos")]
    let gpu_tag = "AGX";
    #[cfg(not(target_os = "macos"))]
    let gpu_tag = "DEV";
    rows.push(BoxRow {
        text: format!(
            " {gpu_c}●{RESET} {BOLD}{WHITE}GPU{RESET} {DIM}{gpu_tag}{RESET}  {BOLD}{gpu_c}{:4.1}%{RESET}  {DIM}vram {:.1}G{RESET}",
            sampler.gpu_pct, sampler.gpu_vram_gb
        ),
        bg: "",
        tag: None,
    });
    for cl in render_multirow_curve(&sampler.gpu_hist, curve_w, h_gpu, BLUE, CYAN, AMBER) {
        rows.push(BoxRow {
            text: format!(" {}", cl),
            bg: "",
            tag: None,
        });
    }
    if use_dividers {
        rows.push(BoxRow {
            text: div_str.clone(),
            bg: "",
            tag: None,
        });
    }

    // 3. MEM
    let mem_c = if sampler.mem_pct >= 85.0 {
        CORAL
    } else if sampler.mem_pct >= 70.0 {
        AMBER
    } else {
        LAVENDER
    };
    rows.push(BoxRow {
        text: format!(
            " {mem_c}●{RESET} {BOLD}{WHITE}MEM{RESET} {DIM}{:.0}G{RESET}  {BOLD}{mem_c}{:4.1}%{RESET}  {DIM}{:.1}/{:.0}G{RESET}",
            sampler.total_mem_gb, sampler.mem_pct, sampler.mem_used_gb, sampler.total_mem_gb
        ),
        bg: "",
        tag: None,
    });
    for cl in render_multirow_curve(&sampler.mem_hist, curve_w, h_mem, LAVENDER, AMBER, CORAL) {
        rows.push(BoxRow {
            text: format!(" {}", cl),
            bg: "",
            tag: None,
        });
    }
    if use_dividers {
        rows.push(BoxRow {
            text: div_str,
            bg: "",
            tag: None,
        });
    }

    // 4. NET
    let rx_s = fmt_rate(sampler.net_rx_bps);
    let tx_s = fmt_rate(sampler.net_tx_bps);
    rows.push(BoxRow {
        text: format!(
            " {CYAN}●{RESET} {BOLD}{WHITE}NET{RESET} {GREEN}▼{:>4}{RESET} {AMBER}▲{:>4}{RESET} {DIM}pk {}/s{RESET}",
            rx_s,
            tx_s,
            fmt_rate(sampler.net_peak_bps)
        ),
        bg: "",
        tag: None,
    });
    for cl in render_multirow_curve(&sampler.net_hist, curve_w, h_net, CYAN, MINT, GREEN) {
        rows.push(BoxRow {
            text: format!(" {}", cl),
            bg: "",
            tag: None,
        });
    }

    rows.truncate(avail);
    rows
}
