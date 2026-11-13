//! Linux `/proc` & `/sys` Hardware Telemetry Sampler (`x86_64` / `aarch64`).

use std::fs;
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use crate::ffi::getloadavg;
use crate::platform::ipc_path;
use crate::telemetry::SharedTelemetrySnapshot;

pub fn detect_hw_info() -> (String, usize, f64) {
    let mut brand = "LINUX".to_string();
    let mut cores = 0usize;
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("processor") {
                cores += 1;
            } else if (line.starts_with("model name") || line.starts_with("Hardware"))
                && brand == "LINUX"
            {
                if let Some((_, val)) = line.split_once(':') {
                    let v = val.trim();
                    let up = v.to_ascii_uppercase();
                    brand = if up.contains("EPYC") {
                        "AMD EPYC".to_string()
                    } else if up.contains("RYZEN") {
                        "AMD RYZEN".to_string()
                    } else if up.contains("XEON") {
                        "INTEL XEON".to_string()
                    } else {
                        v.chars().take(12).collect()
                    };
                }
            }
        }
    }
    if cores == 0 {
        cores = 8;
    }
    let mut mem_gb = 16.0;
    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Some(kb_str) = rest.split_whitespace().next() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        mem_gb = (kb as f64) / (1024.0 * 1024.0);
                    }
                }
                break;
            }
        }
    }
    (brand, cores, mem_gb)
}

pub fn sample_metrics(
    snap: &mut SharedTelemetrySnapshot,
    total_mem_gb: f64,
    dt_s: f64,
    now_ms: u64,
) {
    if let Ok(stat) = fs::read_to_string("/proc/stat") {
        if let Some(first) = stat.lines().next() {
            if first.starts_with("cpu ") {
                let nums: Vec<u64> = first
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|x| x.parse::<u64>().ok())
                    .collect();
                if nums.len() >= 4 {
                    let user = nums[0];
                    let nice = nums[1];
                    let sys = nums[2]
                        + nums.get(5).copied().unwrap_or(0)
                        + nums.get(6).copied().unwrap_or(0)
                        + nums.get(7).copied().unwrap_or(0);
                    let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
                    let t = [user as u32, sys as u32, idle as u32, nice as u32];
                    if snap.prev_cpu_ticks != [0; 4] {
                        let du = t[0].wrapping_sub(snap.prev_cpu_ticks[0]) as u64;
                        let ds = t[1].wrapping_sub(snap.prev_cpu_ticks[1]) as u64;
                        let di = t[2].wrapping_sub(snap.prev_cpu_ticks[2]) as u64;
                        let dn = t[3].wrapping_sub(snap.prev_cpu_ticks[3]) as u64;
                        let tot = du + ds + di + dn;
                        if tot > 0 {
                            snap.cpu_pct =
                                (100.0 * ((du + ds + dn) as f64) / (tot as f64)).clamp(0.0, 100.0);
                        }
                    }
                    snap.prev_cpu_ticks = t;
                }
            }
        }
    }

    unsafe {
        let mut lavg = [0.0f64; 3];
        if getloadavg(lavg.as_mut_ptr(), 3) > 0 {
            snap.load_1m = lavg[0];
        }
    }

    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        let mut total_kb = 0u64;
        let mut avail_kb = 0u64;
        for line in meminfo.lines() {
            if let Some(r) = line.strip_prefix("MemTotal:") {
                total_kb = r
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            } else if let Some(r) = line.strip_prefix("MemAvailable:") {
                avail_kb = r
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
            if total_kb > 0 && avail_kb > 0 {
                break;
            }
        }
        if total_kb > 0 {
            let used_kb = total_kb.saturating_sub(avail_kb);
            snap.mem_used_gb = (used_kb as f64) / (1024.0 * 1024.0);
            snap.mem_pct = (100.0 * snap.mem_used_gb / total_mem_gb.max(1.0)).clamp(0.0, 100.0);
        }
    }

    if let Ok(netdev) = fs::read_to_string("/proc/net/dev") {
        let mut rx_b = 0u64;
        let mut tx_b = 0u64;
        for line in netdev.lines().skip(2) {
            if let Some((iface, rest)) = line.split_once(':') {
                if iface.trim() == "lo" {
                    continue;
                }
                let cols: Vec<&str> = rest.split_whitespace().collect();
                if cols.len() >= 9 {
                    rx_b = rx_b.saturating_add(cols[0].parse::<u64>().unwrap_or(0));
                    tx_b = tx_b.saturating_add(cols[8].parse::<u64>().unwrap_or(0));
                }
            }
        }
        if snap.prev_net_rx > 0 || snap.prev_net_tx > 0 {
            snap.net_rx_bps = (rx_b.saturating_sub(snap.prev_net_rx) as f64) / dt_s;
            snap.net_tx_bps = (tx_b.saturating_sub(snap.prev_net_tx) as f64) / dt_s;
        }
        snap.prev_net_rx = rx_b;
        snap.prev_net_tx = tx_b;
    }

    let mut got_sysfs_gpu = false;
    if let Ok(busy) = fs::read_to_string("/sys/class/drm/card0/device/gpu_busy_percent") {
        if let Ok(pct) = busy.trim().parse::<f64>() {
            snap.gpu_pct = pct.clamp(0.0, 100.0);
            got_sysfs_gpu = true;
        }
    }
    if !got_sysfs_gpu && std::path::Path::new("/proc/driver/nvidia/version").exists() {
        let nv_cache = ipc_path("nvidia_stat.txt");
        if let Ok(txt) = fs::read_to_string(&nv_cache) {
            if let Some((u_s, m_s)) = txt.lines().next().and_then(|l| l.split_once(',')) {
                if let Ok(u) = u_s.trim().parse::<f64>() {
                    snap.gpu_pct = u.clamp(0.0, 100.0);
                }
                if let Ok(mb) = m_s.trim().parse::<f64>() {
                    snap.gpu_vram_gb = mb / 1024.0;
                }
            }
        }
        let need_spawn = fs::metadata(&nv_cache)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| now_ms.saturating_sub(d.as_millis() as u64) > 1500)
            .unwrap_or(true);
        if need_spawn {
            if let Ok(out_file) = fs::File::create(&nv_cache) {
                let _ = Command::new("nvidia-smi")
                    .args([
                        "--query-gpu=utilization.gpu,memory.used",
                        "--format=csv,noheader,nounits",
                    ])
                    .stdout(Stdio::from(out_file))
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
    }
}
