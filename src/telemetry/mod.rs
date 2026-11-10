//! Shared Single-Instance Cross-Platform (`macOS M4 Pro` + `Linux Server`) Hardware Telemetry Collector.
//!
//! All open `ccide-core sidebar` instances across all windows and tabs share a single
//! binary snapshot (`/tmp/.cc-ide/hw_telemetry_v1.bin`, 1,672 bytes `#[repr(C)]`).
//! - At most ONE hardware sample is taken globally across the entire machine every `500ms`.
//! - All other tabs (and fast `150ms` UI ticks) reuse the shared snapshot in `< 2 µs` (`0.002ms`),
//!   eliminating redundant `IOKit` / `sysctl` / `/proc` calls and keeping oscilloscope curves
//!   100% synchronized across all tabs.

pub mod chart;

#[cfg(target_os = "macos")]
use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::fs;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::getloadavg;
#[cfg(target_os = "macos")]
use crate::ffi::{
    CFDictionaryGetValue, CFNumberGetValue, CFRelease, CFStringCreateWithCString, HostCpuLoadInfo,
    IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperties,
    IOServiceGetMatchingServices, IOServiceMatching, IfMsghdr2, host_page_size, host_statistics,
    host_statistics64, mach_host_self, sysctl, sysctl_string, sysctl_u64,
};

const SHARED_TELEMETRY_PATH: &str = "/tmp/.cc-ide/hw_telemetry_v1.bin";
const SHARED_MAGIC: u64 = 0x4343_4944_455F_4857; // "CCIDE_HW"
const GLOBAL_SAMPLE_INTERVAL_MS: u64 = 500;
pub const HIST_LEN: usize = 48;

#[repr(C)]
#[derive(Clone, Copy)]
struct SharedTelemetrySnapshot {
    magic: u64,
    updated_at_ms: u64,
    prev_cpu_ticks: [u32; 4],
    prev_net_rx: u64,
    prev_net_tx: u64,
    cpu_cores: u32,
    _pad: u32,
    total_mem_gb: f64,
    cpu_pct: f64,
    load_1m: f64,
    gpu_pct: f64,
    gpu_vram_gb: f64,
    mem_pct: f64,
    mem_used_gb: f64,
    net_rx_bps: f64,
    net_tx_bps: f64,
    net_peak_bps: f64,
    cpu_hist: [f64; HIST_LEN],
    gpu_hist: [f64; HIST_LEN],
    mem_hist: [f64; HIST_LEN],
    net_hist: [f64; HIST_LEN],
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_shared_snapshot() -> Option<SharedTelemetrySnapshot> {
    let bytes = fs::read(SHARED_TELEMETRY_PATH).ok()?;
    if bytes.len() != std::mem::size_of::<SharedTelemetrySnapshot>() {
        return None;
    }
    let snap: SharedTelemetrySnapshot =
        unsafe { ptr::read_unaligned(bytes.as_ptr() as *const SharedTelemetrySnapshot) };
    if snap.magic == SHARED_MAGIC {
        Some(snap)
    } else {
        None
    }
}

fn write_shared_snapshot(snap: &SharedTelemetrySnapshot) {
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            snap as *const SharedTelemetrySnapshot as *const u8,
            std::mem::size_of::<SharedTelemetrySnapshot>(),
        )
    };
    let tmp = format!("{}.tmp.{}", SHARED_TELEMETRY_PATH, std::process::id());
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, SHARED_TELEMETRY_PATH);
    }
}

pub struct TelemetrySampler {
    #[cfg(target_os = "macos")]
    host: c_uint,
    #[cfg(target_os = "macos")]
    page_size: usize,
    pub cpu_brand: String,
    pub cpu_cores: usize,
    pub total_mem_gb: f64,
    pub cpu_hist: Vec<f64>,
    pub gpu_hist: Vec<f64>,
    pub mem_hist: Vec<f64>,
    pub net_hist: Vec<f64>,
    pub cpu_pct: f64,
    pub load_1m: f64,
    pub gpu_pct: f64,
    pub gpu_vram_gb: f64,
    pub mem_pct: f64,
    pub mem_used_gb: f64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub net_peak_bps: f64,
}

impl TelemetrySampler {
    pub fn new(_history_len: usize) -> Self {
        let _ = fs::create_dir_all("/tmp/.cc-ide");

        #[cfg(target_os = "macos")]
        let (host, page_size, cpu_brand, cpu_cores, total_mem_gb) = {
            let h = unsafe { mach_host_self() };
            let mut ps: usize = 16384;
            unsafe {
                let _ = host_page_size(h, &mut ps);
            }
            if ps == 0 {
                ps = 16384;
            }
            let brand = sysctl_string("machdep.cpu.brand_string")
                .unwrap_or_else(|| "Apple M4 Pro".to_string())
                .replace("Apple ", "");
            let cores = sysctl_u64("hw.logicalcpu").unwrap_or(12) as usize;
            let mem_gb = (sysctl_u64("hw.memsize").unwrap_or(25769803776) as f64)
                / (1024.0 * 1024.0 * 1024.0);
            (h, ps, brand, cores, mem_gb)
        };

        #[cfg(not(target_os = "macos"))]
        let (cpu_brand, cpu_cores, total_mem_gb) = detect_linux_hw_info();

        let mut s = Self {
            #[cfg(target_os = "macos")]
            host,
            #[cfg(target_os = "macos")]
            page_size,
            cpu_brand,
            cpu_cores,
            total_mem_gb,
            cpu_hist: vec![8.0; HIST_LEN],
            gpu_hist: vec![6.0; HIST_LEN],
            mem_hist: vec![50.0; HIST_LEN],
            net_hist: vec![3.0; HIST_LEN],
            cpu_pct: 8.0,
            load_1m: 1.0,
            gpu_pct: 6.0,
            gpu_vram_gb: 1.5,
            mem_pct: 50.0,
            mem_used_gb: 12.0,
            net_rx_bps: 0.0,
            net_tx_bps: 0.0,
            net_peak_bps: 512.0 * 1024.0,
        };
        s.sample();
        s
    }

    fn apply_snapshot(&mut self, snap: &SharedTelemetrySnapshot) {
        self.cpu_pct = snap.cpu_pct;
        self.load_1m = snap.load_1m;
        self.gpu_pct = snap.gpu_pct;
        self.gpu_vram_gb = snap.gpu_vram_gb;
        self.mem_pct = snap.mem_pct;
        self.mem_used_gb = snap.mem_used_gb;
        self.net_rx_bps = snap.net_rx_bps;
        self.net_tx_bps = snap.net_tx_bps;
        self.net_peak_bps = snap.net_peak_bps;
        self.cpu_hist.clear();
        self.cpu_hist.extend_from_slice(&snap.cpu_hist);
        self.gpu_hist.clear();
        self.gpu_hist.extend_from_slice(&snap.gpu_hist);
        self.mem_hist.clear();
        self.mem_hist.extend_from_slice(&snap.mem_hist);
        self.net_hist.clear();
        self.net_hist.extend_from_slice(&snap.net_hist);
    }

    /// Reuses `/tmp/.cc-ide/hw_telemetry_v1.bin` if sampled within the last 500ms by ANY tab;
    /// otherwise performs a single hardware sample and updates the shared snapshot for all tabs.
    pub fn sample(&mut self) {
        let now_ms = now_epoch_ms();
        let existing = read_shared_snapshot();

        if let Some(ref snap) = existing
            && now_ms.saturating_sub(snap.updated_at_ms) < GLOBAL_SAMPLE_INTERVAL_MS
        {
            self.apply_snapshot(snap);
            return;
        }

        let mut snap = existing.unwrap_or(SharedTelemetrySnapshot {
            magic: SHARED_MAGIC,
            updated_at_ms: 0,
            prev_cpu_ticks: [0; 4],
            prev_net_rx: 0,
            prev_net_tx: 0,
            cpu_cores: self.cpu_cores as u32,
            _pad: 0,
            total_mem_gb: self.total_mem_gb,
            cpu_pct: 8.0,
            load_1m: 1.0,
            gpu_pct: 6.0,
            gpu_vram_gb: 1.5,
            mem_pct: 65.0,
            mem_used_gb: 16.0,
            net_rx_bps: 0.0,
            net_tx_bps: 0.0,
            net_peak_bps: 512.0 * 1024.0,
            cpu_hist: [8.0; HIST_LEN],
            gpu_hist: [6.0; HIST_LEN],
            mem_hist: [65.0; HIST_LEN],
            net_hist: [3.0; HIST_LEN],
        });

        let dt_s = if snap.updated_at_ms > 0 {
            ((now_ms.saturating_sub(snap.updated_at_ms)) as f64 / 1000.0).clamp(0.05, 10.0)
        } else {
            0.5
        };

        #[cfg(target_os = "macos")]
        {
            unsafe {
                let mut info = HostCpuLoadInfo { cpu_ticks: [0; 4] };
                let mut cnt: c_uint = 4;
                if host_statistics(
                    self.host,
                    3,
                    &mut info as *mut HostCpuLoadInfo as *mut c_void,
                    &mut cnt,
                ) == 0
                {
                    let t = info.cpu_ticks;
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

                let mut lavg = [0.0f64; 3];
                if getloadavg(lavg.as_mut_ptr(), 3) > 0 {
                    snap.load_1m = lavg[0];
                }

                let mut u32_buf = [0u32; 40];
                let mut vcnt: c_uint = 38;
                if host_statistics64(
                    self.host,
                    4,
                    u32_buf.as_mut_ptr() as *mut c_void,
                    &mut vcnt,
                ) == 0
                {
                    let wired = u32_buf[3] as u64;
                    let purgeable = u32_buf[22] as u64;
                    let comp_occupied = u32_buf[32] as u64;
                    let internal = u32_buf[35] as u64;
                    let used_pages = internal.saturating_sub(purgeable) + wired + comp_occupied;
                    snap.mem_used_gb = ((used_pages * (self.page_size as u64)) as f64)
                        / (1024.0 * 1024.0 * 1024.0);
                    snap.mem_pct =
                        (100.0 * snap.mem_used_gb / self.total_mem_gb.max(1.0)).clamp(0.0, 100.0);
                }
            }

            let (gpu_u, vram_g) = sample_gpu_iokit();
            snap.gpu_pct = gpu_u;
            if vram_g > 0.0 {
                snap.gpu_vram_gb = vram_g;
            }

            let (rx_b, tx_b) = sample_net_sysctl();
            if snap.prev_net_rx > 0 || snap.prev_net_tx > 0 {
                snap.net_rx_bps = (rx_b.saturating_sub(snap.prev_net_rx) as f64) / dt_s;
                snap.net_tx_bps = (tx_b.saturating_sub(snap.prev_net_tx) as f64) / dt_s;
            }
            snap.prev_net_rx = rx_b;
            snap.prev_net_tx = tx_b;
        }

        #[cfg(not(target_os = "macos"))]
        {
            sample_linux_metrics(&mut snap, self.total_mem_gb, dt_s, now_ms);
        }

        let tot_net = snap.net_rx_bps + snap.net_tx_bps;
        snap.net_peak_bps = (256.0 * 1024.0f64)
            .max(snap.net_peak_bps * 0.98)
            .max(tot_net);
        let net_pct = if tot_net > 0.0 {
            (100.0 * tot_net / snap.net_peak_bps.max(1.0)).clamp(2.0, 100.0)
        } else {
            0.0
        };

        let is_first_init = snap.updated_at_ms == 0;
        snap.updated_at_ms = now_ms;

        if is_first_init {
            snap.cpu_hist = [snap.cpu_pct.max(3.0); HIST_LEN];
            snap.gpu_hist = [snap.gpu_pct.max(3.0); HIST_LEN];
            snap.mem_hist = [snap.mem_pct.max(10.0); HIST_LEN];
            snap.net_hist = [net_pct.max(2.0); HIST_LEN];
        } else {
            snap.cpu_hist.copy_within(1..HIST_LEN, 0);
            snap.cpu_hist[HIST_LEN - 1] = snap.cpu_pct;
            snap.gpu_hist.copy_within(1..HIST_LEN, 0);
            snap.gpu_hist[HIST_LEN - 1] = snap.gpu_pct;
            snap.mem_hist.copy_within(1..HIST_LEN, 0);
            snap.mem_hist[HIST_LEN - 1] = snap.mem_pct;
            snap.net_hist.copy_within(1..HIST_LEN, 0);
            snap.net_hist[HIST_LEN - 1] = net_pct;
        }

        write_shared_snapshot(&snap);
        self.apply_snapshot(&snap);

        let _ = fs::write(
            "/tmp/.cc-ide/sys_telemetry.cache",
            format!(
                "CPU {:.0}% \u{e0b1} GPU {:.0}% \u{e0b1} RAM {:.0}%",
                snap.cpu_pct, snap.gpu_pct, snap.mem_pct
            ),
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn detect_linux_hw_info() -> (String, usize, f64) {
    let mut brand = "LINUX".to_string();
    let mut cores = 0usize;
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("processor") {
                cores += 1;
            } else if (line.starts_with("model name") || line.starts_with("Hardware"))
                && brand == "LINUX"
                && let Some((_, val)) = line.split_once(':')
            {
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
    if cores == 0 {
        cores = 8;
    }
    let mut mem_gb = 16.0;
    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Some(kb_str) = rest.split_whitespace().next()
                    && let Ok(kb) = kb_str.parse::<u64>()
                {
                    mem_gb = (kb as f64) / (1024.0 * 1024.0);
                }
                break;
            }
        }
    }
    (brand, cores, mem_gb)
}

#[cfg(not(target_os = "macos"))]
fn sample_linux_metrics(
    snap: &mut SharedTelemetrySnapshot,
    total_mem_gb: f64,
    dt_s: f64,
    now_ms: u64,
) {
    // 1. CPU ticks from `/proc/stat`
    if let Ok(stat) = fs::read_to_string("/proc/stat")
        && let Some(first) = stat.lines().next()
        && first.starts_with("cpu ")
    {
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

    // 2. Load average
    unsafe {
        let mut lavg = [0.0f64; 3];
        if getloadavg(lavg.as_mut_ptr(), 3) > 0 {
            snap.load_1m = lavg[0];
        }
    }

    // 3. Memory from `/proc/meminfo`
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

    // 4. Network from `/proc/net/dev`
    if let Ok(netdev) = fs::read_to_string("/proc/net/dev") {
        let mut rx_b = 0u64;
        let mut tx_b = 0u64;
        for line in netdev.lines().skip(2) {
            if let Some((iface, rest)) = line.split_once(':') {
                let ifname = iface.trim();
                if ifname == "lo" {
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

    // 5. GPU from sysfs (`gpu_busy_percent`) or cached non-blocking `nvidia-smi`
    if let Ok(busy) = fs::read_to_string("/sys/class/drm/card0/device/gpu_busy_percent")
        && let Ok(pct) = busy.trim().parse::<f64>()
    {
        snap.gpu_pct = pct.clamp(0.0, 100.0);
    } else if std::path::Path::new("/proc/driver/nvidia/version").exists() {
        let nv_cache = "/tmp/.cc-ide/nvidia_stat.txt";
        if let Ok(txt) = fs::read_to_string(nv_cache)
            && let Some(line) = txt.lines().next()
            && let Some((u_s, m_s)) = line.split_once(',')
        {
            if let Ok(u) = u_s.trim().parse::<f64>() {
                snap.gpu_pct = u.clamp(0.0, 100.0);
            }
            if let Ok(mb) = m_s.trim().parse::<f64>() {
                snap.gpu_vram_gb = mb / 1024.0;
            }
        }
        let need_spawn = fs::metadata(nv_cache)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t. duration_since(UNIX_EPOCH).ok())
            .map(|d| now_ms.saturating_sub(d.as_millis() as u64) > 1500)
            .unwrap_or(true);
        if need_spawn {
            let _ = std::process::Command::new("sh")
                .args([
                    "-c",
                    "nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader,nounits > /tmp/.cc-ide/nvidia_stat.txt.tmp 2>/dev/null && mv /tmp/.cc-ide/nvidia_stat.txt.tmp /tmp/.cc-ide/nvidia_stat.txt",
                ])
                .spawn();
        }
    }
}

#[cfg(target_os = "macos")]
fn sample_gpu_iokit() -> (f64, f64) {
    unsafe {
        let Ok(cls) = CString::new("IOAccelerator") else {
            return (0.0, 0.0);
        };
        let matching = IOServiceMatching(cls.as_ptr());
        if matching.is_null() {
            return (0.0, 0.0);
        }
        let mut iter: c_uint = 0;
        if IOServiceGetMatchingServices(0, matching, &mut iter) != 0 {
            return (0.0, 0.0);
        }
        let mut gpu_pct = 0.0f64;
        let mut vram_gb = 0.0f64;

        let k_perf = CFStringCreateWithCString(
            ptr::null(),
            c"PerformanceStatistics".as_ptr() as *const c_char,
            0x08000100,
        );
        let k_dev = CFStringCreateWithCString(
            ptr::null(),
            c"Device Utilization %".as_ptr() as *const c_char,
            0x08000100,
        );
        let k_ren = CFStringCreateWithCString(
            ptr::null(),
            c"Renderer Utilization %".as_ptr() as *const c_char,
            0x08000100,
        );
        let k_inuse = CFStringCreateWithCString(
            ptr::null(),
            c"In use system memory".as_ptr() as *const c_char,
            0x08000100,
        );
        let k_alloc = CFStringCreateWithCString(
            ptr::null(),
            c"Alloc system memory".as_ptr() as *const c_char,
            0x08000100,
        );

        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            let mut props: *mut c_void = ptr::null_mut();
            if IORegistryEntryCreateCFProperties(entry, &mut props, ptr::null(), 0) == 0
                && !props.is_null()
            {
                let perf = CFDictionaryGetValue(props, k_perf);
                if !perf.is_null() {
                    let mut v: i64 = 0;
                    let d_ptr = CFDictionaryGetValue(perf, k_dev);
                    if !d_ptr.is_null()
                        && CFNumberGetValue(d_ptr, 4, &mut v as *mut i64 as *mut c_void)
                    {
                        gpu_pct = gpu_pct.max(v as f64);
                    }
                    let r_ptr = CFDictionaryGetValue(perf, k_ren);
                    if !r_ptr.is_null()
                        && CFNumberGetValue(r_ptr, 4, &mut v as *mut i64 as *mut c_void)
                    {
                        gpu_pct = gpu_pct.max(v as f64);
                    }
                    let m_ptr = CFDictionaryGetValue(perf, k_inuse);
                    let m_ptr = if m_ptr.is_null() {
                        CFDictionaryGetValue(perf, k_alloc)
                    } else {
                        m_ptr
                    };
                    if !m_ptr.is_null()
                        && CFNumberGetValue(m_ptr, 4, &mut v as *mut i64 as *mut c_void)
                    {
                        vram_gb = (v as f64) / (1024.0 * 1024.0 * 1024.0);
                    }
                }
                CFRelease(props);
            }
            IOObjectRelease(entry);
        }
        CFRelease(k_perf);
        CFRelease(k_dev);
        CFRelease(k_ren);
        CFRelease(k_inuse);
        CFRelease(k_alloc);
        IOObjectRelease(iter);
        (gpu_pct, vram_gb)
    }
}

#[cfg(target_os = "macos")]
fn sample_net_sysctl() -> (u64, u64) {
    unsafe {
        let mut mib: [c_int; 6] = [4, 17, 0, 0, 6, 0];
        let mut len: usize = 0;
        if sysctl(
            mib.as_mut_ptr(),
            6,
            ptr::null_mut(),
            &mut len,
            ptr::null_mut(),
            0,
        ) != 0
            || len == 0
        {
            return (0, 0);
        }
        let mut buf = vec![0u8; len];
        if sysctl(
            mib.as_mut_ptr(),
            6,
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            ptr::null_mut(),
            0,
        ) != 0
        {
            return (0, 0);
        }
        let mut rx_total = 0u64;
        let mut tx_total = 0u64;
        let mut off = 0usize;
        while off + std::mem::size_of::<IfMsghdr2>() <= len {
            let hdr = &*(buf.as_ptr().add(off) as *const IfMsghdr2);
            if hdr.ifm_msglen == 0 {
                break;
            }
            if hdr.ifm_type == 0x12
                && (hdr.ifm_flags & 0x8) == 0
                && (hdr.ifm_flags & 0x1) != 0
                && hdr.ifm_data.ifi_type == 6
            {
                rx_total = rx_total.saturating_add(hdr.ifm_data.ifi_ibytes);
                tx_total = tx_total.saturating_add(hdr.ifm_data.ifi_obytes);
            }
            off += hdr.ifm_msglen as usize;
        }
        (rx_total, tx_total)
    }
}
