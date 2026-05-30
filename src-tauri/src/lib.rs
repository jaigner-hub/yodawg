//! yodawg — a friendly QEMU wrapper.
//!
//! Tauri backend: owns VM configs on disk, spawns/tracks `qemu-system-*`
//! processes, and exposes lifecycle commands to the React frontend.

mod qemu;
mod qmp;
mod vm;

use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use vm::VmConfig;

/// A currently-running VM: its child process and the ports it was launched with.
struct RunningVm {
    child: Child,
    vnc_display: u16,
    websocket_port: u16,
    qmp_port: u16,
}

/// App-wide runtime state: name -> running process.
#[derive(Default)]
struct AppState {
    running: Mutex<HashMap<String, RunningVm>>,
}

/// Connection info the frontend needs to attach the noVNC viewer.
#[derive(Serialize, Clone)]
struct RunningInfo {
    name: String,
    websocket_port: u16,
    vnc_display: u16,
    qmp_port: u16,
}

/// A VM plus whether it is currently running — the unit the VM list renders.
#[derive(Serialize)]
struct VmStatus {
    #[serde(flatten)]
    config: VmConfig,
    running: bool,
    websocket_port: Option<u16>,
}

/// Resolve the app data directory, creating it if needed.
fn app_data(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Drop entries whose process has exited so stale VMs don't read as running.
fn prune_dead(running: &mut HashMap<String, RunningVm>) {
    let dead: Vec<String> = running
        .iter_mut()
        .filter_map(|(name, rv)| match rv.child.try_wait() {
            Ok(Some(_)) => Some(name.clone()), // exited
            _ => None,
        })
        .collect();
    for name in dead {
        running.remove(&name);
    }
}

// ----------------------------------------------------------------------------
// Commands
// ----------------------------------------------------------------------------

/// QEMU version string, or an error if QEMU can't be found/run.
#[tauri::command]
fn detect_qemu() -> Result<String, String> {
    qemu::detect()
}

/// List all VMs with their running status.
#[tauri::command]
fn list_vms(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<VmStatus>, String> {
    let dir = app_data(&app)?;
    let mut running = state.running.lock().unwrap();
    prune_dead(&mut running);

    let vms = vm::load_all(&dir)
        .into_iter()
        .map(|config| {
            let rv = running.get(&config.name);
            VmStatus {
                running: rv.is_some(),
                websocket_port: rv.map(|r| r.websocket_port),
                config,
            }
        })
        .collect();
    Ok(vms)
}

/// Create a new VM: write its config and create its disk image.
#[tauri::command]
fn create_vm(
    app: AppHandle,
    name: String,
    memory_mb: u32,
    cpus: u32,
    disk_size_gb: u32,
    iso_path: Option<String>,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("VM name cannot be empty".into());
    }
    if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("VM name contains invalid characters".into());
    }
    let dir = app_data(&app)?;
    if vm::machine_dir(&dir, &name).exists() {
        return Err(format!("A VM named '{name}' already exists"));
    }

    let disk_path = vm::machine_dir(&dir, &name)
        .join("disk.qcow2")
        .to_string_lossy()
        .into_owned();

    let cfg = VmConfig {
        name,
        memory_mb,
        cpus,
        disk_path: disk_path.clone(),
        disk_size_gb,
        iso_path: iso_path.filter(|s| !s.is_empty()),
    };

    // Persist config first (creates the directory), then create the disk in it.
    vm::save(&dir, &cfg)?;
    qemu::create_disk(&disk_path, disk_size_gb)?;
    Ok(())
}

/// Update an existing VM's editable settings (memory, CPUs, attached ISO).
/// Changes take effect on the next start.
#[tauri::command]
fn update_vm(
    app: AppHandle,
    name: String,
    memory_mb: u32,
    cpus: u32,
    iso_path: Option<String>,
) -> Result<(), String> {
    if memory_mb < 64 {
        return Err("Memory must be at least 64 MB".into());
    }
    if cpus < 1 {
        return Err("Need at least 1 CPU".into());
    }
    let dir = app_data(&app)?;
    let mut cfg = vm::load(&dir, &name)?;
    cfg.memory_mb = memory_mb;
    cfg.cpus = cpus;
    cfg.iso_path = iso_path.filter(|s| !s.is_empty());
    vm::save(&dir, &cfg)
}

/// Detach the install ISO from a VM so it boots from its disk only. Takes
/// effect on the next start.
#[tauri::command]
fn detach_iso(app: AppHandle, name: String) -> Result<(), String> {
    let dir = app_data(&app)?;
    let mut cfg = vm::load(&dir, &name)?;
    cfg.iso_path = None;
    vm::save(&dir, &cfg)
}

/// Delete a stopped VM and its files.
#[tauri::command]
fn delete_vm(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<(), String> {
    {
        let mut running = state.running.lock().unwrap();
        prune_dead(&mut running);
        if running.contains_key(&name) {
            return Err("Stop the VM before deleting it".into());
        }
    }
    let dir = app_data(&app)?;
    vm::delete(&dir, &name)
}

/// Start a VM. Spawns QEMU and returns the connection info for the viewer.
#[tauri::command]
fn start_vm(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<RunningInfo, String> {
    let dir = app_data(&app)?;
    let cfg = vm::load(&dir, &name)?;

    let mut running = state.running.lock().unwrap();
    prune_dead(&mut running);
    if running.contains_key(&name) {
        return Err(format!("'{name}' is already running"));
    }

    let ports = qemu::LaunchPorts {
        vnc_display: qemu::free_vnc_display()?,
        websocket_port: qemu::free_port()?,
        qmp_port: qemu::free_port()?,
    };
    let args = qemu::build_args(&cfg, &ports);

    let child = std::process::Command::new(qemu::system_binary())
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to launch QEMU: {e}"))?;

    let info = RunningInfo {
        name: name.clone(),
        websocket_port: ports.websocket_port,
        vnc_display: ports.vnc_display,
        qmp_port: ports.qmp_port,
    };
    running.insert(
        name,
        RunningVm {
            child,
            vnc_display: ports.vnc_display,
            websocket_port: ports.websocket_port,
            qmp_port: ports.qmp_port,
        },
    );
    Ok(info)
}

/// Connection info for a running VM, or `None` if it isn't running.
#[tauri::command]
fn running_info(state: State<'_, AppState>, name: String) -> Option<RunningInfo> {
    let mut running = state.running.lock().unwrap();
    prune_dead(&mut running);
    running.get(&name).map(|rv| RunningInfo {
        name: name.clone(),
        websocket_port: rv.websocket_port,
        vnc_display: rv.vnc_display,
        qmp_port: rv.qmp_port,
    })
}

/// Gracefully shut down a VM via QMP (ACPI power button). The guest OS decides
/// when to actually power off; the process exits on its own and `prune_dead`
/// reaps it.
#[tauri::command]
fn stop_vm(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let qmp_port = {
        let running = state.running.lock().unwrap();
        running
            .get(&name)
            .map(|rv| rv.qmp_port)
            .ok_or_else(|| format!("'{name}' is not running"))?
    };
    qmp::execute(qmp_port, "system_powerdown")?;
    Ok(())
}

/// Forcibly terminate a VM's process (the "pull the plug" option).
#[tauri::command]
fn force_kill_vm(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let mut running = state.running.lock().unwrap();
    if let Some(mut rv) = running.remove(&name) {
        rv.child.kill().map_err(|e| e.to_string())?;
        let _ = rv.child.wait();
        Ok(())
    } else {
        Err(format!("'{name}' is not running"))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            detect_qemu,
            list_vms,
            create_vm,
            update_vm,
            detach_iso,
            delete_vm,
            start_vm,
            running_info,
            stop_vm,
            force_kill_vm,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
