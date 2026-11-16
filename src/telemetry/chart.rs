//! Multi-Row (`8 × height` Sub-Pixel) Oscilloscope Curve Renderer & Telemetry Panel Builder.

use crate::telemetry::TelemetrySampler;
use crate::tui::grid::BoxRow;
use crate::tui::layout::RowLayout;
use crate::tui::palette::Palette;
use crate::tui::{BOLD, RESET, SPARK_BLOCKS};

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
pub fn render_multirow_curve<'a>(
    samples: &[f64],
    width: usize,
    height: usize,
    col_low: &'a str,
    col_mid: &'a str,
    col_high: &'a str,
    grid_dot: &str,
) -> Vec<String> {
    let (height, width) = (height.max(1), width.max(8));
    let sliced = &samples[samples.len().saturating_sub(width)..];
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
                    s.push_str(grid_dot);
                } else {
                    s.push(' ');
                }
            } else {
                let ch = SPARK_BLOCKS[(rem.min(8)) as usize];
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

/// Builds exact-height `BoxRow`s for the bottom Telemetry Panel (`CPU`, `GPU`, `MEM`, `NET`).
pub fn build_telemetry_panel_rows(
    sampler: &TelemetrySampler,
    inner_w: usize,
    panel_height: usize,
    pal: &Palette,
) -> Vec<BoxRow> {
    let avail = panel_height.saturating_sub(2).max(8);
    let curve_w = inner_w.saturating_sub(2).max(10);
    let use_dividers = avail >= 15;
    let curve_budget = avail
        .saturating_sub(if use_dividers { 7 } else { 4 })
        .max(4);

    let h_cpu = (curve_budget / 4 + usize::from(curve_budget % 4 >= 1)).max(1);
    let h_gpu = (curve_budget / 4 + usize::from(curve_budget % 4 >= 2)).max(1);
    let h_mem = (curve_budget / 4 + usize::from(curve_budget % 4 >= 3)).max(1);
    let h_net = curve_budget.saturating_sub(h_cpu + h_gpu + h_mem).max(1);

    let (white, dim, cyan, green, amber, coral, blue, mint, lavender, grid_dot) = (
        &pal.text,
        &pal.dim,
        &pal.cyan,
        &pal.green,
        &pal.amber,
        &pal.coral,
        &pal.blue,
        &pal.mint,
        &pal.lavender,
        &pal.grid_dot,
    );
    let div_str = format!(" {}{}{RESET}", pal.divider, "┄".repeat(curve_w));
    let row_fmt = RowLayout::new(inner_w + 2);
    let mut rows = Vec::with_capacity(avail);

    let push_sec = |rows: &mut Vec<BoxRow>,
                    left: String,
                    right: String,
                    hist: &[f64],
                    h: usize,
                    c_lo: &str,
                    c_md: &str,
                    c_hi: &str,
                    div: Option<&str>| {
        rows.push(BoxRow {
            text: row_fmt.split(&left, &right, 1),
            bg: String::new(),
            tag: None,
        });
        for cl in render_multirow_curve(hist, curve_w, h, c_lo, c_md, c_hi, grid_dot) {
            rows.push(BoxRow {
                text: format!(" {cl}"),
                bg: String::new(),
                tag: None,
            });
        }
        if let Some(d) = div {
            rows.push(BoxRow {
                text: d.to_string(),
                bg: String::new(),
                tag: None,
            });
        }
    };

    let cpu_c = if sampler.cpu_pct >= 80.0 {
        coral
    } else if sampler.cpu_pct >= 45.0 {
        amber
    } else {
        mint
    };
    let div_opt = use_dividers.then_some(div_str.as_str());
    push_sec(
        &mut rows,
        format!(
            "{cpu_c}●{RESET} {BOLD}{white}CPU{RESET} {dim}{}C{RESET}  {BOLD}{cpu_c}{:4.1}%{RESET}",
            sampler.cpu_cores, sampler.cpu_pct
        ),
        format!("{dim}load {:.2}{RESET}", sampler.load_1m),
        &sampler.cpu_hist,
        h_cpu,
        mint,
        cyan,
        coral,
        div_opt,
    );

    let gpu_c = if sampler.gpu_pct >= 80.0 {
        coral
    } else if sampler.gpu_pct >= 45.0 {
        amber
    } else {
        blue
    };
    #[cfg(target_os = "macos")]
    let gpu_tag = "AGX";
    #[cfg(not(target_os = "macos"))]
    let gpu_tag = "DEV";
    push_sec(
        &mut rows,
        format!(
            "{gpu_c}●{RESET} {BOLD}{white}GPU{RESET} {dim}{gpu_tag}{RESET}  {BOLD}{gpu_c}{:4.1}%{RESET}",
            sampler.gpu_pct
        ),
        format!("{dim}vram {:.1}G{RESET}", sampler.gpu_vram_gb),
        &sampler.gpu_hist,
        h_gpu,
        blue,
        cyan,
        amber,
        div_opt,
    );

    let mem_c = if sampler.mem_pct >= 85.0 {
        coral
    } else if sampler.mem_pct >= 70.0 {
        amber
    } else {
        lavender
    };
    push_sec(
        &mut rows,
        format!(
            "{mem_c}●{RESET} {BOLD}{white}MEM{RESET} {dim}{:.0}G{RESET}  {BOLD}{mem_c}{:4.1}%{RESET}",
            sampler.total_mem_gb, sampler.mem_pct
        ),
        format!("{dim}{:.1}/{:.0}G{RESET}", sampler.mem_used_gb, sampler.total_mem_gb),
        &sampler.mem_hist,
        h_mem,
        lavender,
        amber,
        coral,
        div_opt,
    );

    push_sec(
        &mut rows,
        format!(
            "{cyan}●{RESET} {BOLD}{white}NET{RESET} {green}▼{:>4}{RESET} {amber}▲{:>4}{RESET}",
            fmt_rate(sampler.net_rx_bps),
            fmt_rate(sampler.net_tx_bps)
        ),
        format!("{dim}pk {}/s{RESET}", fmt_rate(sampler.net_peak_bps)),
        &sampler.net_hist,
        h_net,
        cyan,
        mint,
        green,
        None,
    );

    rows.truncate(avail);
    rows
}
