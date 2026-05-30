//! Process utilities for VMs that outlive the app.
//!
//! yodawg deliberately lets QEMU keep running when the app closes, so a relaunch
//! can reattach (see `session.rs`). Because a reattached VM was spawned by a
//! previous process, we no longer hold a `std::process::Child` for it — we track
//! it by PID and need to query liveness / terminate it by PID instead.

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };

    const STILL_ACTIVE: u32 = 259;

    /// Whether a process with this PID is still running.
    pub fn pid_alive(pid: u32) -> bool {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false; // can't open -> gone (or not ours)
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            // STILL_ACTIVE means the process hasn't exited. (A process that
            // genuinely exits with code 259 is misread as alive, but QEMU
            // doesn't, and the QMP identity check is the real guard anyway.)
            ok != 0 && code == STILL_ACTIVE
        }
    }

    /// Forcibly terminate a process by PID. Best-effort.
    pub fn kill_pid(pid: u32) {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use std::process::Command;

    pub fn pid_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn kill_pid(pid: u32) {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

pub use imp::{kill_pid, pid_alive};
