//! Shared Single-Instance Cross-Platform (`macOS`, `Linux`, `Windows`) Hardware Telemetry (`src/telemetry/`).

pub mod chart;
#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use std::ffi::c_uint;
use std::fs;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::platform::{ipc_dir, ipc_path};

const SHARED_MAGIC: u64 = 0x4343_4944_455F_4857; // "CCIDE_HW"
const GLOBAL_SAMPLE_INTERVAL_MS: u64 = 500;
pub const HIST_LEN: usize = 48;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SharedTelemetrySnapshot {
    pub magic: u64,
    pub updated_at_ms: u64,
    pub prev_cpu_ticks: [u32; 4],
    pub prev_net_rx: u64,
    pub prev_net_tx: u64,
    pub cpu_cores: u32,
    pub _pad: u32,
    pub total_mem_gb: f64,
    pub cpu_pct: f64,
    pub load_1m: f64,
    pub gpu_pct: f64,
    pub gpu_vram_gb: f64,
    pub mem_pct: f64,
    pub mem_used_gb: f64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub net_peak_bps: f64,
    pub cpu_hist: [f64; HIST_LEN],
    pub gpu_hist: [f64; HIST_LEN],
    pub mem_hist: [f64; HIST_LEN],
    pub net_hist: [f64; HIST_LEN],
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_shared_snapshot() -> Option<SharedTelemetrySnapshot> {
    let bytes = fs::read(ipc_path("hw_telemetry_v1.bin")).ok()?;
    if bytes.len() != std::mem::size_of::<SharedTelemetrySnapshot>() {
        return None;
    }
    let snap: SharedTelemetrySnapshot =
        unsafe { ptr::read_unaligned(bytes.as_ptr() as *const SharedTelemetrySnapshot) };
    (snap.magic == SHARED_MAGIC).then_some(snap)
}

fn write_shared_snapshot(snap: &SharedTelemetrySnapshot) {
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            snap as *const SharedTelemetrySnapshot as *const u8,
            std::mem::size_of::<SharedTelemetrySnapshot>(),
        )
    };
    let dst = ipc_path("hw_telemetry_v1.bin");
    let tmp = ipc_path(&format!("hw_telemetry_v1.bin.tmp.{}", std::process::id()));
    if fs::write(&tmp, bytes).is_ok() {
        let mut renamed = fs::rename(&tmp, &dst).is_ok();
        if !renamed {
            for _ in 0..3 {
                std::thread::sleep(std::time::Duration::from_millis(2));
                if fs::rename(&tmp, &dst).is_ok() {
                    renamed = true;
                    break;
                }
            }
        }
        if !renamed {
            let _ = fs::remove_file(&tmp);
        }
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
        let _ = fs::create_dir_all(ipc_dir());

        #[cfg(target_os = "macos")]
        let (host, page_size, cpu_brand, cpu_cores, total_mem_gb) = macos::detect_hw_info();
        #[cfg(all(unix, not(target_os = "macos")))]
        let (cpu_brand, cpu_cores, total_mem_gb) = linux::detect_hw_info();
        #[cfg(windows)]
        let (cpu_brand, cpu_cores, total_mem_gb) = windows::detect_hw_info();

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

    pub fn sample(&mut self) {
        let now_ms = now_epoch_ms();
        let existing = read_shared_snapshot();

        if let Some(ref snap) = existing {
            if now_ms.saturating_sub(snap.updated_at_ms) < GLOBAL_SAMPLE_INTERVAL_MS {
                self.apply_snapshot(snap);
                return;
            }
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
        macos::sample_metrics(
            self.host,
            self.page_size,
            self.total_mem_gb,
            &mut snap,
            dt_s,
        );

        #[cfg(all(unix, not(target_os = "macos")))]
        linux::sample_metrics(&mut snap, self.total_mem_gb, dt_s, now_ms);

        #[cfg(windows)]
        windows::sample_metrics(&mut snap, self.total_mem_gb, dt_s, now_ms);

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
            ipc_path("sys_telemetry.cache"),
            format!(
                "CPU {:.0}% \u{e0b1} GPU {:.0}% \u{e0b1} RAM {:.0}%",
                snap.cpu_pct, snap.gpu_pct, snap.mem_pct
            ),
        );
    }
}

#[cfg(target_os = "macos")]
impl Drop for TelemetrySampler {
    fn drop(&mut self) {
        macos::release_host_port(self.host);
    }
}
