//! Tie spawned child processes to the lifetime of this process.
//!
//! On Windows, std's `Child` does not kill its children when the parent dies.
//! If the app crashes or is force-closed, spawned QEMU processes would be
//! orphaned — still holding RAM, disk locks, and ports. We prevent that with a
//! Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: every child
//! assigned to the job is terminated when the last handle to the job closes,
//! which happens automatically when our process exits, however it exits.
//!
//! On non-Windows platforms this is a no-op (the same guarantee is achieved
//! differently and isn't needed for the current Windows target).

#[cfg(target_os = "windows")]
mod imp {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct KillOnExitJob(HANDLE);

    // The job handle is owned for the whole process lifetime and the Win32 job
    // APIs are thread-safe, so it's safe to share across threads.
    unsafe impl Send for KillOnExitJob {}
    unsafe impl Sync for KillOnExitJob {}

    impl KillOnExitJob {
        pub fn new() -> Option<Self> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    return None;
                }
                Some(KillOnExitJob(job))
            }
        }

        /// Assign a freshly spawned child to the job. Best-effort: a failure
        /// only loses the kill-on-exit guarantee for that one process.
        pub fn assign(&self, child: &Child) {
            unsafe {
                AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use std::process::Child;

    pub struct KillOnExitJob;

    impl KillOnExitJob {
        pub fn new() -> Option<Self> {
            Some(KillOnExitJob)
        }
        pub fn assign(&self, _child: &Child) {}
    }
}

pub use imp::KillOnExitJob;
