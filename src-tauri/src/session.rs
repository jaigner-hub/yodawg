//! Persistence of VMs left running in the background.
//!
//! When the app closes it no longer powers guests down — they keep running.
//! This records each running VM's PID and ports to `<app_data>/running.json` so
//! a relaunch can probe them and reattach (see `lib.rs::reconcile_running`)
//! instead of losing track of them.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Enough to find a previously-launched QEMU and reconnect its control/display.
#[derive(Serialize, Deserialize, Clone)]
pub struct PersistedVm {
    pub name: String,
    pub pid: u32,
    pub qmp_port: u16,
    /// SPICE port the external virt-viewer client connects to. Defaults to 0
    /// for records written before this field existed (those VMs were launched
    /// without a SPICE server, so the viewer can't connect — a relaunch of the
    /// VM gets a real port). Older records also carried `vnc_display` /
    /// `websocket_port`; those keys are now ignored on load.
    #[serde(default)]
    pub spice_port: u16,
}

fn path(app_data: &Path) -> PathBuf {
    app_data.join("running.json")
}

/// Overwrite the running-VM record with the current set.
pub fn save(app_data: &Path, vms: &[PersistedVm]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(vms).map_err(|e| e.to_string())?;
    fs::write(path(app_data), json).map_err(|e| e.to_string())
}

/// Load the last-recorded running VMs (empty if none / unreadable).
pub fn load(app_data: &Path) -> Vec<PersistedVm> {
    match fs::read_to_string(path(app_data)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}
