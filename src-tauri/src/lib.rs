//! yodawg — a friendly QEMU wrapper.
//!
//! Tauri backend: owns VM configs on disk, spawns/tracks `qemu-system-*`
//! processes, and exposes lifecycle commands to the React frontend.

mod proc_job;
mod qemu;
mod qmp;
mod vm;

use proc_job::KillOnExitJob;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
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
struct AppState {
    running: Mutex<HashMap<String, RunningVm>>,
    /// Kills any assigned QEMU child if the app process dies (Windows).
    job: Option<KillOnExitJob>,
    /// Set once a graceful "shut everything down then exit" is in progress, so
    /// the window-close handler doesn't kick it off twice.
    shutting_down: AtomicBool,
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
    display_adapter: String,
    port_forwards: Vec<vm::PortForward>,
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
        display_adapter,
        port_forwards,
    };

    // Persist config first (creates the directory), then create the disk in it.
    vm::save(&dir, &cfg)?;
    qemu::create_disk(&disk_path, disk_size_gb)?;
    Ok(())
}

/// Update an existing VM's editable settings (memory, CPUs, ISO, display
/// adapter, port forwards). Changes take effect on the next start.
#[tauri::command]
fn update_vm(
    app: AppHandle,
    name: String,
    memory_mb: u32,
    cpus: u32,
    iso_path: Option<String>,
    display_adapter: String,
    port_forwards: Vec<vm::PortForward>,
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
    cfg.display_adapter = display_adapter;
    cfg.port_forwards = port_forwards;
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

    // Quick "already running?" check (don't hold the lock across the spawn/wait).
    {
        let mut running = state.running.lock().unwrap();
        prune_dead(&mut running);
        if running.contains_key(&name) {
            return Err(format!("'{name}' is already running"));
        }
    }

    let ports = qemu::LaunchPorts {
        vnc_display: qemu::free_vnc_display()?,
        websocket_port: qemu::free_port()?,
        qmp_port: qemu::free_port()?,
    };
    let args = qemu::build_args(&cfg, &ports);

    let mut child = std::process::Command::new(qemu::system_binary())
        .args(&args)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch QEMU: {e}"))?;

    // Tie the child to our lifetime so it can never be orphaned.
    if let Some(job) = &state.job {
        job.assign(&child);
    }

    // Bad args / missing acceleration usually make QEMU exit within a moment.
    // Give it a beat, and if it died, surface its stderr instead of silently
    // flipping the VM back to "stopped".
    std::thread::sleep(Duration::from_millis(600));
    if let Ok(Some(status)) = child.try_wait() {
        let mut msg = String::new();
        if let Some(mut err) = child.stderr.take() {
            use std::io::Read;
            let _ = err.read_to_string(&mut msg);
        }
        let msg = msg.trim();
        return Err(if msg.is_empty() {
            format!("QEMU exited immediately ({status}).")
        } else {
            format!("QEMU failed to start ({status}):\n{msg}")
        });
    }

    // Still running: drain stderr in the background so the pipe can't fill and
    // block QEMU.
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut sink = String::new();
            let _ = err.read_to_string(&mut sink);
        });
    }

    let info = RunningInfo {
        name: name.clone(),
        websocket_port: ports.websocket_port,
        vnc_display: ports.vnc_display,
        qmp_port: ports.qmp_port,
    };
    {
        let mut running = state.running.lock().unwrap();
        if running.contains_key(&name) {
            // Lost a race with another start; don't leak this process.
            let _ = child.kill();
            return Err(format!("'{name}' is already running"));
        }
        running.insert(
            name,
            RunningVm {
                child,
                vnc_display: ports.vnc_display,
                websocket_port: ports.websocket_port,
                qmp_port: ports.qmp_port,
            },
        );
    }
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

/// Gracefully power down every running VM, wait for them to exit, then force
/// kill any stragglers. Blocking — runs on a dedicated thread during app close.
fn shutdown_all_blocking(app: &AppHandle) {
    let state = app.state::<AppState>();

    // Snapshot the QMP ports, then send the ACPI power button to each guest.
    let ports: Vec<u16> = {
        let running = state.running.lock().unwrap();
        running.values().map(|rv| rv.qmp_port).collect()
    };
    for port in ports {
        let _ = qmp::execute(port, "system_powerdown");
    }

    // Give guests up to ~20s to flush and power off cleanly.
    for _ in 0..40 {
        {
            let mut running = state.running.lock().unwrap();
            prune_dead(&mut running);
            if running.is_empty() {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // Anything still alive gets the plug pulled (the Job Object is the final
    // backstop if we never even get here).
    let mut running = state.running.lock().unwrap();
    for rv in running.values_mut() {
        let _ = rv.child.kill();
    }
    running.clear();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState {
        running: Mutex::new(HashMap::new()),
        job: KillOnExitJob::new(),
        shutting_down: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // When the user closes the window with VMs running, hold the close,
            // shut the guests down cleanly on a background thread, then exit.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let state = app.state::<AppState>();

                let any_running = {
                    let mut running = state.running.lock().unwrap();
                    prune_dead(&mut running);
                    !running.is_empty()
                };
                if !any_running {
                    return; // nothing to do; let it close
                }
                if state.shutting_down.swap(true, Ordering::SeqCst) {
                    return; // shutdown already underway
                }

                api.prevent_close();
                let app2 = app.clone();
                std::thread::spawn(move || {
                    shutdown_all_blocking(&app2);
                    app2.exit(0);
                });
            }
        })
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
