//! The Win32 calls behind the Windows git stop, made only on the runner's own git and the
//! processes it started: list processes with their parents, ask one to exit through an injected
//! thread that runs ExitProcess (so its atexit handlers run and git removes its lock files), and
//! terminate one. Every call fails soft; a process that cannot be opened or asked is left for
//! the caller's kill. All of this crate's unsafe code lives here.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetCurrentProcess, GetProcessTimes, IsWow64Process, OpenProcess,
    PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE, TerminateProcess, WaitForSingleObject,
};
use windows_sys::core::{s, w};

/// The exit code a stopped process reports, the one a shell gives a process ended by TERM.
const EXIT_CODE: u32 = 128 + 15;

/// What CreateRemoteThread needs to ask, wait on and terminate a process.
const ACCESS: u32 = PROCESS_CREATE_THREAD
    | PROCESS_QUERY_INFORMATION
    | PROCESS_VM_OPERATION
    | PROCESS_VM_READ
    | PROCESS_VM_WRITE
    | PROCESS_SYNCHRONIZE
    | PROCESS_TERMINATE;

/// A thread start routine, the shape ExitProcess is called through.
type StartRoutine = unsafe extern "system" fn(*mut c_void) -> u32;

/// An open process, held so its id cannot be reused until this is dropped.
pub(super) struct Process {
    handle: HANDLE,
    pid: u32,
    created: u64,
}

impl Process {
    /// Opens `pid` with the rights to ask, wait on and terminate it; `None` when it is gone or
    /// not ours to open.
    pub(super) fn open(pid: u32) -> Option<Process> {
        // SAFETY: OpenProcess takes no pointers, and a null result is checked before use.
        let handle = unsafe { OpenProcess(ACCESS, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut process = Process {
            handle,
            pid,
            created: 0,
        };
        let mut times = [FILETIME::default(); 4];
        let [created, exited, kernel, user] = &mut times;
        // SAFETY: the handle is open with PROCESS_QUERY_INFORMATION, and each pointer is a
        // live, writable FILETIME.
        if unsafe { GetProcessTimes(handle, created, exited, kernel, user) } == 0 {
            return None;
        }
        process.created =
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        Some(process)
    }

    pub(super) fn pid(&self) -> u32 {
        self.pid
    }

    /// When the process started, in FILETIME units.
    pub(super) fn created(&self) -> u64 {
        self.created
    }

    pub(super) fn running(&self) -> bool {
        // SAFETY: the handle is open with SYNCHRONIZE, and a zero timeout never blocks.
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
    }

    /// Starts a thread in the process that calls ExitProcess, as Git for Windows' own kill
    /// does; false when ExitProcess's address cannot be known to hold there or no thread starts.
    pub(super) fn ask_to_exit(&self) -> bool {
        let Some(start) = exit_process() else {
            return false;
        };
        if !same_architecture(self.handle) {
            return false;
        }
        // SAFETY: the handle is open with the rights CreateRemoteThread needs. kernel32 sits at
        // the same address in every process of one architecture, so `start` is ExitProcess in
        // the target too, and its argument is the exit code, passed by value, not a pointer.
        let thread = unsafe {
            CreateRemoteThread(
                self.handle,
                ptr::null(),
                0,
                Some(start),
                ptr::without_provenance(EXIT_CODE as usize),
                0,
                ptr::null_mut(),
            )
        };
        if thread.is_null() {
            return false;
        }
        // SAFETY: `thread` is the handle CreateRemoteThread just returned, closed once here.
        unsafe { CloseHandle(thread) };
        true
    }

    pub(super) fn terminate(&self) {
        // SAFETY: the handle is open with PROCESS_TERMINATE; a failure means it already ended.
        unsafe { TerminateProcess(self.handle, EXIT_CODE) };
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcess, and only this drop closes it.
        unsafe { CloseHandle(self.handle) };
    }
}

/// Every running process's id and its parent's id; empty when the snapshot fails.
pub(super) fn processes() -> Vec<(u32, u32)> {
    // SAFETY: CreateToolhelp32Snapshot takes no pointers; failure is checked below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut list = Vec::new();
    // SAFETY: the snapshot is open, and `entry` is a live PROCESSENTRY32W whose dwSize names
    // its own size, as both walk calls require.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        list.push((entry.th32ProcessID, entry.th32ParentProcessID));
        // SAFETY: as for Process32FirstW above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the snapshot handle was opened above and is closed once here.
    unsafe { CloseHandle(snapshot) };
    list
}

/// ExitProcess in this process's kernel32, looked up once.
fn exit_process() -> Option<StartRoutine> {
    static ADDRESS: OnceLock<Option<StartRoutine>> = OnceLock::new();
    *ADDRESS.get_or_init(|| {
        // SAFETY: the name is a NUL-terminated wide literal; kernel32 is loaded in every
        // process and never unloaded, so the handle needs no release.
        let kernel32 = unsafe { GetModuleHandleW(w!("kernel32.dll")) };
        if kernel32.is_null() {
            return None;
        }
        // SAFETY: the module handle is valid (checked above) and the name is NUL-terminated.
        let address = unsafe { GetProcAddress(kernel32, s!("ExitProcess")) }?;
        // SAFETY: ExitProcess takes one 32-bit code and never returns, so a thread may start
        // in it: the pointer-sized start argument carries the code in the register or stack
        // slot ExitProcess reads it from, and both use the system calling convention.
        Some(unsafe {
            mem::transmute::<unsafe extern "system" fn() -> isize, StartRoutine>(address)
        })
    })
}

/// Whether `process` runs under the same WOW64 emulation as this one, so this process's
/// ExitProcess address holds there too; false when either cannot be told.
fn same_architecture(process: HANDLE) -> bool {
    let wow64 = |handle: HANDLE| {
        let mut flag = 0;
        // SAFETY: the handle is the current-process pseudo handle or one open with
        // PROCESS_QUERY_INFORMATION, and `flag` is a live, writable BOOL.
        (unsafe { IsWow64Process(handle, &mut flag) } != 0).then_some(flag != 0)
    };
    // SAFETY: GetCurrentProcess takes nothing and returns a pseudo handle that needs no close.
    let current = unsafe { GetCurrentProcess() };
    matches!((wow64(current), wow64(process)), (Some(a), Some(b)) if a == b)
}
