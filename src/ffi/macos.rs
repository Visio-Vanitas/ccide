//! Darwin (`macOS arm64 / x86_64`) Mach, BSD `sysctl`, `libproc` & `IOKit` FFI Bindings.

use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::ptr;

#[repr(C)]
pub struct HostCpuLoadInfo {
    pub cpu_ticks: [u32; 4],
}

#[repr(C)]
pub struct Timeval32 {
    pub tv_sec: i32,
    pub tv_usec: i32,
}

#[repr(C)]
pub struct IfData64 {
    pub ifi_type: u8,
    pub ifi_typelen: u8,
    pub ifi_physical: u8,
    pub ifi_addrlen: u8,
    pub ifi_hdrlen: u8,
    pub ifi_recvquota: u8,
    pub ifi_xmitquota: u8,
    pub ifi_unused1: u8,
    pub ifi_mtu: u32,
    pub ifi_metric: u32,
    pub ifi_baudrate: u64,
    pub ifi_ipackets: u64,
    pub ifi_ierrors: u64,
    pub ifi_opackets: u64,
    pub ifi_oerrors: u64,
    pub ifi_collisions: u64,
    pub ifi_ibytes: u64,
    pub ifi_obytes: u64,
    pub ifi_imcasts: u64,
    pub ifi_omcasts: u64,
    pub ifi_iqdrops: u64,
    pub ifi_noproto: u64,
    pub ifi_recvtiming: u32,
    pub ifi_xmittiming: u32,
    pub ifi_lastchange: Timeval32,
}

#[repr(C)]
pub struct IfMsghdr2 {
    pub ifm_msglen: u16,
    pub ifm_version: u8,
    pub ifm_type: u8,
    pub ifm_addrs: c_int,
    pub ifm_flags: c_int,
    pub ifm_index: u16,
    pub _pad: u16,
    pub ifm_snd_len: c_int,
    pub ifm_snd_maxlen: c_int,
    pub ifm_snd_drops: c_int,
    pub ifm_timer: c_int,
    pub ifm_data: IfData64,
}

#[link(name = "System")]
extern "C" {
    pub fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
    pub fn mach_host_self() -> c_uint;
    pub fn host_page_size(host: c_uint, out_page_size: *mut usize) -> c_int;
    pub fn host_statistics(
        host: c_uint,
        flavor: c_int,
        info: *mut c_void,
        count: *mut c_uint,
    ) -> c_int;
    pub fn host_statistics64(
        host: c_uint,
        flavor: c_int,
        info: *mut c_void,
        count: *mut c_uint,
    ) -> c_int;
    pub fn sysctl(
        name: *mut c_int,
        namelen: c_uint,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
    pub fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

#[allow(clippy::duplicated_attributes)]
#[link(name = "IOKit", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    pub fn IOServiceGetMatchingServices(
        main_port: c_uint,
        matching: *mut c_void,
        existing: *mut c_uint,
    ) -> c_int;
    pub fn IOIteratorNext(iterator: c_uint) -> c_uint;
    pub fn IOObjectRelease(object: c_uint) -> c_int;
    pub fn IORegistryEntryCreateCFProperties(
        entry: c_uint,
        properties: *mut *mut c_void,
        allocator: *const c_void,
        options: u32,
    ) -> c_int;
    pub fn CFDictionaryGetValue(the_dict: *const c_void, key: *const c_void) -> *const c_void;
    pub fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    pub fn CFNumberGetValue(number: *const c_void, the_type: isize, value_ptr: *mut c_void)
        -> bool;
    pub fn CFRelease(cf: *const c_void);
}

pub fn get_ppid_of(pid: u32) -> Option<u32> {
    unsafe {
        let mut buf = [0u32; 34]; // 136 bytes = sizeof(proc_bsdinfo), pbi_ppid at index 4
        let ret = proc_pidinfo(pid as c_int, 3, 0, buf.as_mut_ptr() as *mut c_void, 136);
        if ret <= 0 {
            None
        } else {
            Some(buf[4])
        }
    }
}

pub fn sysctl_string(name: &str) -> Option<String> {
    unsafe {
        let c_name = CString::new(name).ok()?;
        let mut len: usize = 0;
        if sysctlbyname(
            c_name.as_ptr(),
            ptr::null_mut(),
            &mut len,
            ptr::null_mut(),
            0,
        ) != 0
            || len == 0
        {
            return None;
        }
        let mut buf = vec![0u8; len];
        if sysctlbyname(
            c_name.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8(buf).ok()
    }
}

pub fn sysctl_u64(name: &str) -> Option<u64> {
    unsafe {
        let c_name = CString::new(name).ok()?;
        let mut val: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        if sysctlbyname(
            c_name.as_ptr(),
            &mut val as *mut u64 as *mut c_void,
            &mut len,
            ptr::null_mut(),
            0,
        ) == 0
        {
            Some(val)
        } else {
            None
        }
    }
}
