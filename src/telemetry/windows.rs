//! Native Win32 (`kernel32` + `iphlpapi`) CPU, RAM, Network & GPU Telemetry Sampler.

use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use crate::platform::{find_executable, ipc_path, user_home_dir};
use crate::telemetry::SharedTelemetrySnapshot;

#[repr(C)]
#[derive(Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn to_u64(self) -> u64 {
        ((self.high as u64) << 32) | (self.low as u64)
    }
}

#[repr(C)]
struct MemoryStatusEx {
    dw_length: u32,
    dw_memory_load: u32,
    ull_total_phys: u64,
    ull_avail_phys: u64,
    ull_total_page_file: u64,
    ull_avail_page_file: u64,
    ull_total_virtual: u64,
    ull_avail_virtual: u64,
    ull_avail_extended_virtual: u64,
}

#[repr(C)]
struct SystemInfo {
    w_processor_architecture: u16,
    w_reserved: u16,
    dw_page_size: u32,
    lp_minimum_application_address: *mut std::ffi::c_void,
    lp_maximum_application_address: *mut std::ffi::c_void,
    dw_active_processor_mask: usize,
    dw_number_of_processors: u32,
    dw_processor_type: u32,
    dw_allocation_granularity: u32,
    w_processor_level: u16,
    w_processor_revision: u16,
}

#[repr(C)]
struct MibIfRow {
    wsz_name: [u16; 256],
    dw_index: u32,
    dw_type: u32,
    dw_mtu: u32,
    dw_speed: u32,
    dw_phys_addr_len: u32,
    b_phys_addr: [u8; 8],
    dw_admin_status: u32,
    dw_oper_status: u32,
    dw_last_change: u32,
    dw_in_octets: u32,
    dw_in_ucast_pkts: u32,
    dw_in_nucast_pkts: u32,
    dw_in_discards: u32,
    dw_in_errors: u32,
    dw_in_unknown_protos: u32,
    dw_out_octets: u32,
    dw_out_ucast_pkts: u32,
    dw_out_nucast_pkts: u32,
    dw_out_discards: u32,
    dw_out_errors: u32,
    dw_out_qlen: u32,
    dw_descr_len: u32,
    b_descr: [u8; 256],
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemTimes(
        lp_idle_time: *mut FileTime,
        lp_kernel_time: *mut FileTime,
        lp_user_time: *mut FileTime,
    ) -> i32;
    fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
    fn GetSystemInfo(lp_system_info: *mut SystemInfo);
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetIfTable(p_if_table: *mut u8, pdw_size: *mut u32, b_order: i32) -> u32;
}

pub fn detect_hw_info() -> (String, usize, f64) {
    let raw_id = env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "WINDOWS CPU".to_string());
    let up = raw_id.to_ascii_uppercase();
    let brand = if up.contains("RYZEN") {
        "AMD RYZEN".to_string()
    } else if up.contains("EPYC") {
        "AMD EPYC".to_string()
    } else if up.contains("INTEL") {
        "INTEL CORE".to_string()
    } else if up.contains("SNAPDRAGON") || up.contains("ARM") {
        "WIN ARM64".to_string()
    } else {
        "WIN HOST".to_string()
    };

    let (cores, mem_gb) = unsafe {
        let mut sys: SystemInfo = std::mem::zeroed();
        GetSystemInfo(&mut sys);
        let c = (sys.dw_number_of_processors as usize).max(4);

        let mut mem: MemoryStatusEx = std::mem::zeroed();
        mem.dw_length = std::mem::size_of::<MemoryStatusEx>() as u32;
        let gb = if GlobalMemoryStatusEx(&mut mem) != 0 && mem.ull_total_phys > 0 {
            (mem.ull_total_phys as f64) / (1024.0 * 1024.0 * 1024.0)
        } else {
            16.0
        };
        (c, gb)
    };
    (brand, cores, mem_gb)
}

pub fn sample_metrics(
    snap: &mut SharedTelemetrySnapshot,
    total_mem_gb: f64,
    dt_s: f64,
    now_ms: u64,
) {
    unsafe {
        let mut idle = FileTime { low: 0, high: 0 };
        let mut kern = FileTime { low: 0, high: 0 };
        let mut user = FileTime { low: 0, high: 0 };
        if GetSystemTimes(&mut idle, &mut kern, &mut user) != 0 {
            let i_ms = (idle.to_u64() / 10_000) as u32;
            let k_ms = (kern.to_u64() / 10_000) as u32;
            let u_ms = (user.to_u64() / 10_000) as u32;
            let sys_only = k_ms.wrapping_sub(i_ms);
            let cur = [u_ms, sys_only, i_ms, 0];
            if snap.prev_cpu_ticks != [0; 4] {
                let du = cur[0].wrapping_sub(snap.prev_cpu_ticks[0]) as u64;
                let ds = cur[1].wrapping_sub(snap.prev_cpu_ticks[1]) as u64;
                let di = cur[2].wrapping_sub(snap.prev_cpu_ticks[2]) as u64;
                let tot = du + ds + di;
                if tot > 0 {
                    snap.cpu_pct = (100.0 * ((du + ds) as f64) / (tot as f64)).clamp(0.0, 100.0);
                    snap.load_1m = (snap.cpu_pct / 100.0) * (snap.cpu_cores.max(1) as f64);
                }
            }
            snap.prev_cpu_ticks = cur;
        }

        let mut mem: MemoryStatusEx = std::mem::zeroed();
        mem.dw_length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if GlobalMemoryStatusEx(&mut mem) != 0 && mem.ull_total_phys > 0 {
            let used = mem.ull_total_phys.saturating_sub(mem.ull_avail_phys);
            snap.mem_used_gb = (used as f64) / (1024.0 * 1024.0 * 1024.0);
            snap.mem_pct = (100.0 * snap.mem_used_gb / total_mem_gb.max(1.0)).clamp(0.0, 100.0);
        }

        let mut sz = 0u32;
        let _ = GetIfTable(std::ptr::null_mut(), &mut sz, 0);
        if sz >= 4 {
            let mut buf = vec![0u8; sz as usize];
            if GetIfTable(buf.as_mut_ptr(), &mut sz, 0) == 0 {
                let count = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                let row_sz = std::mem::size_of::<MibIfRow>();
                let mut rx = 0u64;
                let mut tx = 0u64;
                for i in 0..count {
                    let off = 4 + i * row_sz;
                    if off + row_sz > buf.len() {
                        break;
                    }
                    let row = std::ptr::read_unaligned(buf.as_ptr().add(off) as *const MibIfRow);
                    if row.dw_type != 24 {
                        rx = rx.saturating_add(u64::from(row.dw_in_octets));
                        tx = tx.saturating_add(u64::from(row.dw_out_octets));
                    }
                }
                if snap.prev_net_rx > 0 || snap.prev_net_tx > 0 {
                    snap.net_rx_bps = (rx.saturating_sub(snap.prev_net_rx) as f64) / dt_s;
                    snap.net_tx_bps = (tx.saturating_sub(snap.prev_net_tx) as f64) / dt_s;
                }
                snap.prev_net_rx = rx;
                snap.prev_net_tx = tx;
            }
        }
    }

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
        .map(|d| now_ms.saturating_sub(d.as_millis() as u64) > 2000)
        .unwrap_or(true);
    if need_spawn {
        if let Some(nvsmi) = find_executable("nvidia-smi", &user_home_dir()) {
            if let Ok(out_file) = fs::File::create(&nv_cache) {
                if let Ok(mut child) = Command::new(nvsmi)
                    .args([
                        "--query-gpu=utilization.gpu,memory.used",
                        "--format=csv,noheader,nounits",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::from(out_file))
                    .stderr(Stdio::null())
                    .spawn()
                {
                    let _ = std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
            }
        }
    }
}
