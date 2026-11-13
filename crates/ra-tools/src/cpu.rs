//! The builds' share of the machine's processor. A build runs in a Windows job object whose CPU
//! rate is hard-capped, and the cap follows the number of builds running: one build may use the
//! whole budget, two half of it each, three a third. Every build reads the count again every two
//! seconds, so the shares move as builds start and end. The person's rule of 2026-11-12: "1
//! builder 90% cpu, 3 at 30% each … it needs to be dynamic, changing as the current number of
//! jobs changes".
//!
//! The rate comes from the caller ([`set_rate`]): this crate does not know the build slots. With
//! no rate set, or off Windows, a build runs uncapped.
//!
//! The same machinery keeps a process's workers from outliving it ([`own_children`]): a job that
//! kills what is in it when it closes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The CPU rate a build may use right now, in hundredths of a percent of the whole machine
/// (9000 is 90%).
pub type Rate = Box<dyn Fn() -> u32 + Send + Sync>;

static RATE: OnceLock<Rate> = OnceLock::new();

/// How often a build reads its share again.
const POLL: Duration = Duration::from_secs(2);

/// Sets the rate every build of this process is capped at; the first call wins.
pub fn set_rate(rate: Rate) {
    let _ = RATE.set(rate);
}

/// A build's share, kept while the build runs: dropped, it stops following the count.
pub struct Share {
    stop: Arc<AtomicBool>,
}

impl Drop for Share {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Caps the process (a handle with the rights to be put in a job) and every process it starts
/// from now on at the build's share, and keeps the cap following the count. `None` when no rate
/// is set or the job cannot be made: the build runs uncapped.
#[cfg(windows)]
#[must_use]
pub fn attach(process: *mut std::ffi::c_void) -> Option<Share> {
    let rate = RATE.get()?;
    // SAFETY: the job handle is made, used and closed here and in the thread that owns it; the
    // process handle is the caller's, valid for the call
    let job = unsafe {
        let job = win::CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        if win::AssignProcessToJobObject(job, process) == 0 {
            win::CloseHandle(job);
            return None;
        }
        job as usize
    };
    let mut last = rate();
    win::cap(job, last);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        while !stopped.load(Ordering::Relaxed) {
            std::thread::sleep(POLL);
            let Some(rate) = RATE.get() else { break };
            let now = rate();
            if now != last {
                win::cap(job, now);
                last = now;
            }
        }
        // a process the build left behind (a daemon) stays in the job: it keeps the whole
        // budget rather than the share of a moment that has passed
        win::cap(job, FULL);
        // SAFETY: the job handle is this thread's alone
        unsafe { win::CloseHandle(job as *mut std::ffi::c_void) };
    });
    Some(Share { stop })
}

/// The whole budget, for what a finished build leaves in its job.
#[cfg(windows)]
const FULL: u32 = 9000;

/// A job whose processes die when it closes: dropped, or with the process holding it, every
/// process put in it, and every process they start, is killed. A walk that dies takes its
/// workers with it, so no worker goes on writing in a worktree the next walk hands to another.
#[cfg(windows)]
pub struct Owner {
    job: usize,
}

#[cfg(windows)]
impl Owner {
    /// A new job that kills its processes when it closes; `None` when the job cannot be made.
    #[must_use]
    pub fn new() -> Option<Self> {
        // SAFETY: the job handle is made here and owned by the value returned
        let job = unsafe { win::CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if job.is_null() {
            return None;
        }
        let owner = Self { job: job as usize };
        win::kill_on_close(owner.job).then_some(owner)
    }

    /// Puts the process (a handle with the rights to be put in a job) in the job; whether it went.
    #[must_use]
    pub fn adopt(&self, process: std::os::windows::io::BorrowedHandle<'_>) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: the job is this value's live handle; the borrowed process handle is open for
        // the call
        unsafe { win::AssignProcessToJobObject(self.job as *mut std::ffi::c_void, process.as_raw_handle()) != 0 }
    }
}

#[cfg(windows)]
impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: the job handle is this value's alone
        unsafe { win::CloseHandle(self.job as *mut std::ffi::c_void) };
    }
}

/// This process's own job, made by the first [`own_children`].
#[cfg(windows)]
static OWNER: OnceLock<Option<Owner>> = OnceLock::new();

/// Makes this process's job that its children die with, once, and has every Claude Code worker
/// it starts from now on join it. The job lives as long as the process: when the process dies,
/// however it dies, the job closes and the workers end with it.
pub fn own_children() {
    #[cfg(windows)]
    if OWNER.get_or_init(Owner::new).is_some() {
        ra_providers::claude_code::set_adopter(adopt);
    }
}

/// Puts the process in this process's own job, when [`own_children`] made one; whether it went.
#[cfg(windows)]
#[must_use]
pub fn adopt(process: std::os::windows::io::BorrowedHandle<'_>) -> bool {
    OWNER.get().and_then(Option::as_ref).is_some_and(|owner| owner.adopt(process))
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    #[repr(C)]
    struct CpuRateControl {
        control_flags: u32,
        cpu_rate: u32,
    }

    const CPU_RATE_CONTROL_INFORMATION: i32 = 15;
    const ENABLE: u32 = 0x1;
    const HARD_CAP: u32 = 0x4;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub(super) fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        pub(super) fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, length: u32) -> i32;
        pub(super) fn CloseHandle(handle: *mut c_void) -> i32;
    }

    /// `JOBOBJECT_BASIC_LIMIT_INFORMATION`.
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        minimum_working_set: usize,
        maximum_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    /// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, its `IO_COUNTERS` as six counts.
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    const EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;

    /// Has the job kill its processes when its last handle closes; whether the limit took.
    pub(super) fn kill_on_close(job: usize) -> bool {
        let info =
            ExtendedLimits { basic: BasicLimits { limit_flags: KILL_ON_JOB_CLOSE, ..BasicLimits::default() }, ..ExtendedLimits::default() };
        // SAFETY: the job is a live handle of the caller's, the struct is the documented layout
        unsafe {
            SetInformationJobObject(
                job as *mut c_void,
                EXTENDED_LIMIT_INFORMATION,
                (&raw const info).cast(),
                u32::try_from(std::mem::size_of::<ExtendedLimits>()).unwrap_or(0),
            ) != 0
        }
    }

    /// Sets the job's hard cap, in hundredths of a percent of the machine.
    pub(super) fn cap(job: usize, rate: u32) {
        let info = CpuRateControl { control_flags: ENABLE | HARD_CAP, cpu_rate: rate.clamp(100, 10_000) };
        // SAFETY: the job is a live handle of the caller's, the struct is the documented layout
        unsafe {
            SetInformationJobObject(
                job as *mut c_void,
                CPU_RATE_CONTROL_INFORMATION,
                (&raw const info).cast(),
                u32::try_from(std::mem::size_of::<CpuRateControl>()).unwrap_or(8),
            );
        }
    }
}
