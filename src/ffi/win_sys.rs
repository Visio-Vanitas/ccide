//! Raw Win32 `kernel32.dll` Constants, `#[repr(C)]` Records & `extern "system"` Bindings.

use std::ffi::c_void;

pub type Handle = *mut c_void;
pub const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
pub const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
pub const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;

pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const STILL_ACTIVE: u32 = 259;
pub const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
pub const EVENT_MODIFY_STATE: u32 = 0x0002;
pub const WAIT_OBJECT_0: u32 = 0;

pub const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
pub const ENABLE_LINE_INPUT: u32 = 0x0002;
pub const ENABLE_ECHO_INPUT: u32 = 0x0004;
pub const ENABLE_WINDOW_INPUT: u32 = 0x0008;
pub const ENABLE_MOUSE_INPUT: u32 = 0x0010;
pub const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
pub const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
pub const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
pub const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
pub const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Coord {
    pub x: i16,
    pub y: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SmallRect {
    pub left: i16,
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
}

#[repr(C)]
pub struct ConsoleScreenBufferInfo {
    pub size: Coord,
    pub cursor_pos: Coord,
    pub attributes: u16,
    pub window: SmallRect,
    pub max_window_size: Coord,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyEventRecord {
    pub key_down: i32,
    pub repeat_count: u16,
    pub virtual_key_code: u16,
    pub virtual_scan_code: u16,
    pub u_char: u16,
    pub control_key_state: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MouseEventRecord {
    pub mouse_pos: Coord,
    pub button_state: u32,
    pub control_key_state: u32,
    pub event_flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union InputRecordEvent {
    pub key: KeyEventRecord,
    pub mouse: MouseEventRecord,
    pub raw: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputRecord {
    pub event_type: u16,
    pub _pad: u16,
    pub event: InputRecordEvent,
}

#[repr(C)]
pub struct ProcessEntry32W {
    pub dw_size: u32,
    pub cnt_usage: u32,
    pub th32_process_id: u32,
    pub th32_default_heap_id: usize,
    pub th32_module_id: u32,
    pub cnt_threads: u32,
    pub th32_parent_process_id: u32,
    pub pc_pri_class_base: i32,
    pub dw_flags: u32,
    pub sz_exe_file: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetStdHandle(n_std_handle: u32) -> Handle;
    pub fn GetConsoleMode(h_console: Handle, lp_mode: *mut u32) -> i32;
    pub fn SetConsoleMode(h_console: Handle, dw_mode: u32) -> i32;
    pub fn GetConsoleScreenBufferInfo(
        h_console_output: Handle,
        lp_info: *mut ConsoleScreenBufferInfo,
    ) -> i32;
    pub fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
    pub fn CreateEventW(
        lp_event_attributes: *mut c_void,
        b_manual_reset: i32,
        b_initial_state: i32,
        lp_name: *const u16,
    ) -> Handle;
    pub fn OpenEventW(dw_desired_access: u32, b_inherit_handle: i32, lp_name: *const u16)
        -> Handle;
    pub fn SetEvent(h_event: Handle) -> i32;
    pub fn WaitForMultipleObjects(
        n_count: u32,
        lp_handles: *const Handle,
        b_wait_all: i32,
        dw_milliseconds: u32,
    ) -> u32;
    pub fn ReadConsoleInputW(
        h_console_input: Handle,
        lp_buffer: *mut InputRecord,
        n_length: u32,
        lp_number_of_events_read: *mut u32,
    ) -> i32;
    pub fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32)
        -> Handle;
    pub fn GetExitCodeProcess(h_process: Handle, lp_exit_code: *mut u32) -> i32;
    pub fn CloseHandle(h_object: Handle) -> i32;
    pub fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> Handle;
    pub fn Process32FirstW(h_snapshot: Handle, lppe: *mut ProcessEntry32W) -> i32;
    pub fn Process32NextW(h_snapshot: Handle, lppe: *mut ProcessEntry32W) -> i32;
}
