//! The processes a card's earlier runs left behind: a test binary still running from the card's
//! target holds its files and its pipes, and the next check of the card waits on it or fails to
//! relink. They are stopped before the card's check, and only the card's own.

use std::path::Path;

/// Stops the processes the card of a worktree left running, this process aside; how many were
/// stopped. The card's are those whose program lies under its own target (`<target>/wt-<id>`),
/// and those run from the project's target, which every tree shares, with their working
/// directory in the worktree, as `cargo test` starts a test binary. A workspace that is not a
/// card's worktree has nothing of its own, and nothing is stopped for it.
#[must_use]
pub fn stop_card_processes(workspace: &Path) -> usize {
    let workspace = std::path::absolute(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let (root, Some(card)) = ra_providers::claude_code::project_of(&workspace) else { return 0 };
    let target = ra_providers::claude_code::project_target(&root);
    let (Some(shared), Some(tree)) = (key(&target), key(&workspace)) else { return 0 };
    let card_target = key(&target.join(format!("wt-{card}")));
    let own = std::process::id();
    let mut stopped = 0;
    for (pid, program) in processes() {
        let Some(program) = key(Path::new(&program)).filter(|_| pid != own) else { continue };
        let of_the_card = card_target.as_ref().is_some_and(|dir| program.starts_with(dir))
            || program.starts_with(&shared)
                && current_dir(pid).and_then(|dir| key(Path::new(&dir))).is_some_and(|dir| dir.starts_with(&tree));
        if of_the_card && stop(pid) {
            stopped += 1;
        }
    }
    stopped
}

/// A path as compared: resolved where it exists (a short name read long), without a verbatim
/// prefix, one separator, case folded on Windows, and ending in a separator so `wt-1` is not the
/// start of `wt-12`.
fn key(path: &Path) -> Option<String> {
    let path = path.canonicalize().or_else(|_| std::path::absolute(path)).ok()?;
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut text = if cfg!(windows) { text.replace('/', "\\").to_lowercase() } else { text.to_owned() };
    let separator = if cfg!(windows) { '\\' } else { '/' };
    if !text.ends_with(separator) {
        text.push(separator);
    }
    Some(text)
}

/// Every process this one may read, by id and the full path of its program.
#[cfg(windows)]
fn processes() -> Vec<(u32, String)> {
    win::processes()
}

/// Every process this one may read, by id and the full path of its program.
#[cfg(target_os = "linux")]
fn processes() -> Vec<(u32, String)> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let program = std::fs::read_link(entry.path().join("exe")).ok()?;
            Some((pid, program.to_string_lossy().into_owned()))
        })
        .collect()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn processes() -> Vec<(u32, String)> {
    Vec::new()
}

/// The working directory of a process; none for one this process may not read.
#[cfg(all(windows, target_pointer_width = "64"))]
fn current_dir(pid: u32) -> Option<String> {
    win::current_dir(pid)
}

/// The working directory of a process; none for one this process may not read.
#[cfg(target_os = "linux")]
fn current_dir(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok().map(|dir| dir.to_string_lossy().into_owned())
}

#[cfg(not(any(all(windows, target_pointer_width = "64"), target_os = "linux")))]
fn current_dir(_pid: u32) -> Option<String> {
    None
}

#[cfg(windows)]
fn stop(pid: u32) -> bool {
    win::terminate(pid)
}

#[cfg(not(windows))]
fn stop(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    /// `PROCESSENTRY32W`.
    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        pid: u32,
        default_heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        base_priority: i32,
        flags: u32,
        exe: [u16; 260],
    }

    /// `PROCESS_BASIC_INFORMATION`, its status widened to the pointer's alignment.
    #[repr(C)]
    struct BasicInformation {
        _exit_status: isize,
        peb: usize,
        _affinity: usize,
        _base_priority: isize,
        _pid: usize,
        _parent: usize,
    }

    const SNAPPROCESS: u32 = 0x2;
    const QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const QUERY_INFORMATION: u32 = 0x400;
    const VM_READ: u32 = 0x10;
    /// Where the 64-bit `PEB` keeps its `ProcessParameters`, and where those keep the
    /// `CurrentDirectory`'s path, a `UNICODE_STRING` (its length in bytes, then its buffer).
    const PARAMETERS_AT: usize = 0x20;
    const CURRENT_DIR_AT: usize = 0x38;
    const CURRENT_DIR_BUFFER_AT: usize = 0x40;
    const TERMINATE: u32 = 0x1;
    const INVALID_HANDLE: *mut c_void = std::ptr::without_provenance_mut(usize::MAX);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(process: *mut c_void, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn TerminateProcess(process: *mut c_void, code: u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn ReadProcessMemory(process: *mut c_void, base: *const c_void, buffer: *mut c_void, size: usize, read: *mut usize) -> i32;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryInformationProcess(process: *mut c_void, class: u32, info: *mut c_void, length: u32, returned: *mut u32) -> i32;
    }

    /// The working directory of a process, read from its parameters; none for one this process
    /// may not open or read.
    #[cfg(target_pointer_width = "64")]
    pub(super) fn current_dir(pid: u32) -> Option<String> {
        // SAFETY: the handle is checked before use and closed once
        let process = unsafe { OpenProcess(QUERY_INFORMATION | VM_READ, 0, pid) };
        if process.is_null() {
            return None;
        }
        let dir = read_current_dir(process);
        // SAFETY: the handle was opened above and is closed once
        unsafe { CloseHandle(process) };
        dir
    }

    #[cfg(target_pointer_width = "64")]
    fn read_current_dir(process: *mut c_void) -> Option<String> {
        let mut info = BasicInformation { _exit_status: 0, peb: 0, _affinity: 0, _base_priority: 0, _pid: 0, _parent: 0 };
        let size = u32::try_from(std::mem::size_of::<BasicInformation>()).ok()?;
        // SAFETY: the buffer is the documented layout and its length is passed
        let status = unsafe { NtQueryInformationProcess(process, 0, (&raw mut info).cast(), size, std::ptr::null_mut()) };
        if status != 0 || info.peb == 0 {
            return None;
        }
        let parameters: usize = read(process, info.peb.checked_add(PARAMETERS_AT)?)?;
        let length: u16 = read(process, parameters.checked_add(CURRENT_DIR_AT)?)?;
        let buffer: usize = read(process, parameters.checked_add(CURRENT_DIR_BUFFER_AT)?)?;
        let mut name = vec![0u16; usize::from(length) / 2];
        let bytes = name.len() * 2;
        // SAFETY: the read is into a buffer of `bytes` bytes, the other process's memory is only read
        let read = unsafe {
            ReadProcessMemory(process, std::ptr::with_exposed_provenance(buffer), name.as_mut_ptr().cast(), bytes, std::ptr::null_mut())
        };
        (read != 0).then(|| String::from_utf16_lossy(&name))
    }

    /// A value of plain data at an address in another process.
    #[cfg(target_pointer_width = "64")]
    fn read<T: Copy + Default>(process: *mut c_void, at: usize) -> Option<T> {
        let mut value = T::default();
        // SAFETY: the read is into one `T`, of plain data, by its size; the other process's
        // memory is only read
        let read = unsafe {
            ReadProcessMemory(
                process,
                std::ptr::with_exposed_provenance(at),
                (&raw mut value).cast(),
                std::mem::size_of::<T>(),
                std::ptr::null_mut(),
            )
        };
        (read != 0).then_some(value)
    }

    /// The ids of the running processes, each with its program's full path where it can be read.
    pub(super) fn processes() -> Vec<(u32, String)> {
        let mut ids = Vec::new();
        // SAFETY: the snapshot is checked before use and closed once; the entry is the documented
        // layout with its size set, as the calls require
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE || snapshot.is_null() {
                return Vec::new();
            }
            let mut entry: ProcessEntry = std::mem::zeroed();
            entry.size = u32::try_from(std::mem::size_of::<ProcessEntry>()).unwrap_or(0);
            let mut more = Process32FirstW(snapshot, &raw mut entry) != 0;
            while more {
                ids.push(entry.pid);
                more = Process32NextW(snapshot, &raw mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        ids.into_iter().filter_map(|pid| image(pid).map(|program| (pid, program))).collect()
    }

    /// The full path of a process's program; none for one this process may not open.
    fn image(pid: u32) -> Option<String> {
        // SAFETY: the handle is checked before use and closed once; the buffer's length is passed
        unsafe {
            let process = OpenProcess(QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut name = [0u16; 1024];
            let mut size = u32::try_from(name.len()).unwrap_or(0);
            let read = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &raw mut size) != 0;
            CloseHandle(process);
            let size = usize::try_from(size).unwrap_or(0).min(name.len());
            read.then(|| String::from_utf16_lossy(&name[..size]))
        }
    }

    /// Ends a process; whether it was ended.
    pub(super) fn terminate(pid: u32) -> bool {
        // SAFETY: the handle is checked before use and closed once
        unsafe {
            let process = OpenProcess(TERMINATE, 0, pid);
            if process.is_null() {
                return false;
            }
            let ended = TerminateProcess(process, 1) != 0;
            CloseHandle(process);
            ended
        }
    }
}
