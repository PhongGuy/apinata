//! The Win32 calls behind the Windows git stop, made only on the runner's own git and the
//! processes it started: find git's process tree, ask a process to stop through a thread
//! injected into it, and terminate it. Every call fails soft; a process that cannot be opened
//! or asked is left for the caller's kill. All of this crate's unsafe code lives here.

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
use windows_sys::core::{PCSTR, s, w};

/// The exit code a stopped process reports, the one a shell gives a process ended by TERM.
const EXIT_CODE: u32 = 128 + 15;

/// CtrlRoutine's argument for Ctrl+C.
const CTRL_C_EVENT: usize = 0;

/// What CreateRemoteThread needs to ask, wait on and terminate a process.
const ACCESS: u32 = PROCESS_CREATE_THREAD
    | PROCESS_QUERY_INFORMATION
    | PROCESS_VM_OPERATION
    | PROCESS_VM_READ
    | PROCESS_VM_WRITE
    | PROCESS_SYNCHRONIZE
    | PROCESS_TERMINATE;

/// A thread start routine, the shape both injected calls are made through.
type StartRoutine = unsafe extern "system" fn(*mut c_void) -> u32;

/// How a process is asked to stop.
#[derive(Clone, Copy)]
pub(super) enum Ask {
    /// Ctrl+C, delivered to this process alone. Git for Windows treats it as SIGINT, whose
    /// handler removes git's lock files before it exits.
    Interrupt,
    /// ExitProcess, which ends the process without its C runtime's atexit cleanup but still
    /// lets its DLLs detach; the step between Ctrl+C and TerminateProcess.
    Exit,
}

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

    pub(super) fn running(&self) -> bool {
        // SAFETY: the handle is open with SYNCHRONIZE, and a zero timeout never blocks.
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
    }

    /// Starts a thread in the process at kernel32's routine for `ask`, as Git for Windows' own
    /// kill does; false when that address cannot be known to hold there or no thread starts.
    pub(super) fn ask(&self, ask: Ask) -> bool {
        let (start, argument) = match ask {
            Ask::Interrupt => (ctrl_routine(), CTRL_C_EVENT),
            Ask::Exit => (exit_process(), EXIT_CODE as usize),
        };
        let Some(start) = start else {
            return false;
        };
        if !same_architecture(self.handle) {
            return false;
        }
        // SAFETY: the handle is open with the rights CreateRemoteThread needs. kernel32 sits at
        // the same address in every process of one architecture, so `start` is the same routine
        // in the target, and its argument is a number passed by value, not a pointer.
        let thread = unsafe {
            CreateRemoteThread(
                self.handle,
                ptr::null(),
                0,
                Some(start),
                ptr::without_provenance(argument),
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

/// Adds to `tree` every running process whose parent is in it, until a pass finds no more. A
/// process must have started after its parent, so a reused parent id is not taken for kinship.
pub(super) fn adopt(tree: &mut Vec<Process>) {
    let all = processes();
    loop {
        let before = tree.len();
        for &(pid, parent) in &all {
            if tree.iter().any(|p| p.pid == pid) {
                continue;
            }
            let Some(born) = tree.iter().find(|p| p.pid == parent).map(|p| p.created) else {
                continue;
            };
            if let Some(process) = Process::open(pid)
                && process.created >= born
            {
                tree.push(process);
            }
        }
        if tree.len() == before {
            break;
        }
    }
}

/// Every running process's id and its parent's id; empty when the snapshot fails.
fn processes() -> Vec<(u32, u32)> {
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

/// The routine behind Ctrl+C, exported by kernel32 or kernelbase depending on the Windows
/// version, looked up once.
fn ctrl_routine() -> Option<StartRoutine> {
    static ADDRESS: OnceLock<Option<StartRoutine>> = OnceLock::new();
    *ADDRESS.get_or_init(|| {
        lookup(w!("kernel32.dll"), s!("CtrlRoutine"))
            .or_else(|| lookup(w!("kernelbase.dll"), s!("CtrlRoutine")))
    })
}

/// ExitProcess in this process's kernel32, looked up once.
fn exit_process() -> Option<StartRoutine> {
    static ADDRESS: OnceLock<Option<StartRoutine>> = OnceLock::new();
    *ADDRESS.get_or_init(|| lookup(w!("kernel32.dll"), s!("ExitProcess")))
}

/// `name` in the already loaded system `module`, called as a thread start routine.
fn lookup(module: *const u16, name: PCSTR) -> Option<StartRoutine> {
    // SAFETY: the module name is a NUL-terminated wide literal; kernel32 and kernelbase are
    // loaded in every process and never unloaded, so the handle needs no release.
    let module = unsafe { GetModuleHandleW(module) };
    if module.is_null() {
        return None;
    }
    // SAFETY: the module handle is valid (checked above) and the name is NUL-terminated.
    let address = unsafe { GetProcAddress(module, name) }?;
    // SAFETY: both routines take one pointer-sized-or-smaller value in the slot a thread's
    // start argument arrives in and use the system calling convention; ExitProcess never
    // returns and CtrlRoutine returns a 32-bit code, so either may start a thread.
    Some(unsafe { mem::transmute::<unsafe extern "system" fn() -> isize, StartRoutine>(address) })
}

/// Whether `process` runs under the same WOW64 emulation as this one, so this process's
/// kernel32 addresses hold there too; false when either cannot be told.
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
