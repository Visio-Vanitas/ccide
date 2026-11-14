//! Reusable Proportional Strip & Token/USD Formatters for `CONTEXT` & `COST`.

use crate::tui::context::{ContextSnapshot, ModelCostEntry};
use crate::tui::palette::Palette;
use crate::tui::RESET;

pub fn fmt_tok_compact(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 10_000 {
        format!("{}k", (tokens + 500) / 1_000)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}

fn fmt_tok_sig(tokens: u64, sig_figs: usize) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    let units: [(f64, char); 3] = [(1e3, 'k'), (1e6, 'M'), (1e9, 'B')];
    let mut u_idx = usize::from(tokens >= 1_000_000) + usize::from(tokens >= 1_000_000_000);
    let digits = |v: f64| 1 + usize::from(v >= 10.0) + usize::from(v >= 100.0);
    loop {
        let val = tokens as f64 / units[u_idx].0;
        let dec = sig_figs.saturating_sub(digits(val));
        let f = 10f64.powi(dec as i32);
        let r = (val * f).round() / f;
        if r >= 1000.0 && u_idx + 1 < units.len() {
            u_idx += 1;
            continue;
        }
        let prec = sig_figs.saturating_sub(digits(r));
        return format!("{:.prec$}{}", r, units[u_idx].1, prec = prec);
    }
}

pub fn fmt_tok_ir(tokens: u64) -> String {
    fmt_tok_sig(tokens, 4)
}

pub fn fmt_tok_o(tokens: u64) -> String {
    fmt_tok_sig(tokens, 3)
}

pub fn fmt_usd(v: f64) -> String {
    if v >= 100.0 {
        format!("${:.0}", v)
    } else if v >= 10.0 {
        format!("${:.1}", v)
    } else if v >= 0.01 || v == 0.0 {
        format!("${:.2}", v)
    } else {
        format!("${:.3}", v)
    }
}

/// High-precision USD formatter (`4–5` significant digits) for the dedicated `COST` panel.
pub fn fmt_usd_precise(v: f64) -> String {
    if v <= 0.0 {
        "$0.00".to_string()
    } else if v >= 1_000.0 {
        format!("${:.1}", v)
    } else if v >= 10.0 {
        format!("${:.2}", v)
    } else if v >= 0.1 {
        format!("${:.3}", v)
    } else if v >= 0.0001 {
        format!("${:.4}", v)
    } else {
        "<$0.0001".to_string()
    }
}

/// Formats model share percentage without misleading `100%` / `0%` rounding when non-zero.
pub fn fmt_share_pct(cost: f64, total_cost: f64, fallback_pct: u64) -> String {
    if total_cost <= 1e-7 {
        return format!("{fallback_pct}%");
    }
    if cost <= 0.0 {
        return "0%".to_string();
    }
    let raw = ((cost / total_cost) * 100.0).clamp(0.0, 100.0);
    if (total_cost - cost) <= 1e-6 || raw >= 99.95 {
        "100%".to_string()
    } else if !(10.0..=99.0).contains(&raw) {
        if raw < 0.1 {
            "<0.1%".to_string()
        } else {
            format!("{:.1}%", raw)
        }
    } else {
        format!("{:.0}%", raw)
    }
}

/// Computes prompt-cache hit ratio to `0.1%` precision, strictly capped at `99.9%` (never `100%`).
pub fn compute_cache_hit_pct(cache_read: u64, total_prompt: u64) -> f64 {
    if total_prompt == 0 || cache_read == 0 {
        return 0.0;
    }
    let raw = (cache_read as f64 / total_prompt as f64) * 100.0;
    ((raw * 10.0).round() / 10.0).clamp(0.0, 99.9)
}

pub fn fmt_tps(tps: Option<f64>) -> String {
    match tps.filter(|&v| v > 0.05 && v.is_finite()) {
        Some(v) if v >= 100.0 => format!("{:.0}", v),
        Some(v) => format!("{:.1}", v),
        None => "--".to_string(),
    }
}

pub fn fmt_ttft(ttft_ms: Option<u64>) -> String {
    match ttft_ms.filter(|&ms| ms > 0) {
        Some(ms) if ms >= 10_000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) if ms >= 1_000 => format!("{:.2}s", ms as f64 / 1000.0),
        Some(ms) => format!("{}ms", ms),
        None => "--".to_string(),
    }
}

pub fn model_family_color<'a>(family: &str, idx_in_family: usize, pal: &'a Palette) -> &'a str {
    if idx_in_family == 0 {
        match family {
            "gemini" => &pal.model_gemini,
            "deepseek" => &pal.model_deepseek,
            "openai" => &pal.model_openai,
            "grok" => &pal.model_grok,
            _ => &pal.model_claude,
        }
    } else {
        match idx_in_family % 4 {
            1 => &pal.lavender,
            2 => &pal.amber,
            3 => &pal.mint,
            _ => &pal.cyan,
        }
    }
}

/// Renders a continuous horizontal proportional strip (`━` colored fill + `━` slate-gray track).
pub fn render_prop_bar(pct_0_to_100: f64, width: usize, fill_col: &str, track_col: &str) -> String {
    let width = width.max(4);
    let clamped = pct_0_to_100.clamp(0.0, 100.0);
    let mut filled = ((clamped / 100.0) * (width as f64)).round() as usize;
    if clamped > 0.4 && filled == 0 {
        filled = 1;
    }
    filled = filled.min(width);
    let empty = width.saturating_sub(filled);
    let mut s = String::with_capacity(width * 6);
    if filled > 0 {
        s.push_str(fill_col);
        for _ in 0..filled {
            s.push('━');
        }
        s.push_str(RESET);
    }
    if empty > 0 {
        s.push_str(track_col);
        for _ in 0..empty {
            s.push('━');
        }
        s.push_str(RESET);
    }
    s
}

fn render_weighted_segments(
    segments: &[(f64, &str)],
    denom: f64,
    width: usize,
    track_col: &str,
) -> String {
    let width = width.max(8);
    let safe_denom = denom.max(1e-9);
    let mut seg_cells = vec![0usize; segments.len()];
    let mut used_cells = 0usize;
    for (i, &(val, _)) in segments.iter().enumerate() {
        if val <= 0.0 {
            continue;
        }
        let raw = ((val / safe_denom) * (width as f64)).round() as usize;
        let c = raw.max(1);
        seg_cells[i] = c;
        used_cells += c;
    }
    while used_cells > width {
        if let Some((max_i, _)) = seg_cells
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v > 1)
            .max_by_key(|&(_, &v)| v)
        {
            seg_cells[max_i] -= 1;
            used_cells -= 1;
        } else {
            break;
        }
    }

    let mut out = String::with_capacity(width * 8);
    let mut written = 0usize;
    for (i, &(_, col)) in segments.iter().enumerate() {
        let n = seg_cells[i].min(width.saturating_sub(written));
        if n > 0 {
            out.push_str(col);
            for _ in 0..n {
                out.push('━');
            }
            written += n;
        }
    }
    if written < width {
        out.push_str(track_col);
        for _ in 0..(width - written) {
            out.push('━');
        }
    }
    out.push_str(RESET);
    out
}

pub fn render_segmented_spectrum_bar(
    snap: &ContextSnapshot,
    width: usize,
    pal: &Palette,
) -> String {
    let denom = snap.budget.limit_tokens.max(snap.total_in_tokens).max(1) as f64;
    let cats: [(f64, &str); 5] = [
        (snap.sys_md_tokens as f64, &pal.lavender),
        (snap.tools_tokens as f64, &pal.blue),
        (snap.tool_io_tokens as f64, &pal.cyan),
        (snap.thinking_tokens as f64, &pal.amber),
        (snap.chat_tokens as f64, &pal.mint),
    ];
    render_weighted_segments(&cats, denom, width, &pal.divider)
}

pub fn render_multi_model_bar(models: &[ModelCostEntry], width: usize, pal: &Palette) -> String {
    let total_cost: f64 = models.iter().map(|m| m.cost_usd).sum();
    let use_cost = total_cost > 1e-7;
    let denom = if use_cost {
        total_cost
    } else {
        models
            .iter()
            .map(|m| m.total_tokens() as f64)
            .sum::<f64>()
            .max(1.0)
    };
    let mut fam_counts: [(&str, usize); 5] = [
        ("claude", 0),
        ("gemini", 0),
        ("deepseek", 0),
        ("openai", 0),
        ("grok", 0),
    ];
    let mut segs: Vec<(f64, &str)> = Vec::with_capacity(models.len());
    for m in models {
        let rank = fam_counts
            .iter_mut()
            .find(|(f, _)| *f == m.family)
            .map(|(_, cnt)| {
                let r = *cnt;
                *cnt += 1;
                r
            })
            .unwrap_or(0);
        let val = if use_cost {
            m.cost_usd
        } else {
            m.total_tokens() as f64
        };
        segs.push((val, model_family_color(m.family, rank, pal)));
    }
    render_weighted_segments(&segs, denom, width, &pal.divider)
}
