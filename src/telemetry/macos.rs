//! Darwin (`macOS`) Mach CPU/VM, `IOKit` GPU & BSD `sysctl` Network Sampler.

use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::ptr;

use crate::ffi::{
    getloadavg, host_page_size, host_statistics, host_statistics64, mach_host_self, sysctl,
    sysctl_string, sysctl_u64, CFDictionaryGetValue, CFNumberGetValue, CFRelease,
    CFStringCreateWithCString, HostCpuLoadInfo, IOIteratorNext, IOObjectRelease,
    IORegistryEntryCreateCFProperties, IOServiceGetMatchingServices, IOServiceMatching, IfMsghdr2,
};
use crate::telemetry::SharedTelemetrySnapshot;

pub fn detect_hw_info() -> (c_uint, usize, String, usize, f64) {
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
    let mem_gb =
        (sysctl_u64("hw.memsize").unwrap_or(25_769_803_776) as f64) / (1024.0 * 1024.0 * 1024.0);
    (h, ps, brand, cores, mem_gb)
}

pub fn sample_metrics(
    host: c_uint,
    page_size: usize,
    total_mem_gb: f64,
    snap: &mut SharedTelemetrySnapshot,
    dt_s: f64,
) {
    unsafe {
        let mut info = HostCpuLoadInfo { cpu_ticks: [0; 4] };
        let mut cnt: c_uint = 4;
        if host_statistics(
            host,
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
        if host_statistics64(host, 4, u32_buf.as_mut_ptr() as *mut c_void, &mut vcnt) == 0 {
            let wired = u32_buf[3] as u64;
            let purgeable = u32_buf[22] as u64;
            let comp_occupied = u32_buf[32] as u64;
            let internal = u32_buf[35] as u64;
            let used_pages = internal.saturating_sub(purgeable) + wired + comp_occupied;
            snap.mem_used_gb =
                ((used_pages * (page_size as u64)) as f64) / (1024.0 * 1024.0 * 1024.0);
            snap.mem_pct = (100.0 * snap.mem_used_gb / total_mem_gb.max(1.0)).clamp(0.0, 100.0);
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

        #[allow(clippy::manual_c_str_literals)]
        let (k_perf, k_dev, k_ren, k_inuse, k_alloc) = (
            CFStringCreateWithCString(
                ptr::null(),
                b"PerformanceStatistics\0".as_ptr() as *const c_char,
                0x08000100,
            ),
            CFStringCreateWithCString(
                ptr::null(),
                b"Device Utilization %\0".as_ptr() as *const c_char,
                0x08000100,
            ),
            CFStringCreateWithCString(
                ptr::null(),
                b"Renderer Utilization %\0".as_ptr() as *const c_char,
                0x08000100,
            ),
            CFStringCreateWithCString(
                ptr::null(),
                b"In use system memory\0".as_ptr() as *const c_char,
                0x08000100,
            ),
            CFStringCreateWithCString(
                ptr::null(),
                b"Alloc system memory\0".as_ptr() as *const c_char,
                0x08000100,
            ),
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
