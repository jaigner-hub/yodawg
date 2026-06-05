//! yodawg — a friendly QEMU wrapper.
//!
//! Tauri backend: owns VM configs on disk, spawns/tracks `qemu-system-*`
//! processes, and exposes lifecycle commands to the React frontend.

mod procutil;
mod qemu;
mod qmp;
mod session;
mod vm;

use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use vm::VmConfig;

/// A currently-running VM and the ports it was launched with.
struct RunningVm {
    /// Owned handle when we launched it this session; `None` when reattached to
    /// a VM a previous session left running (we then track it by `pid`).
    child: Option<Child>,
    pid: u32,
    vnc_display: u16,
    websocket_port: u16,
    qmp_port: u16,
    spice_port: u16,
}

/// App-wide runtime state: name -> running process.
struct AppState {
    running: Mutex<HashMap<String, RunningVm>>,
    /// Set once the "pause guests then exit" close sequence is underway, so the
    /// window-close handler doesn't kick it off twice.
    shutting_down: AtomicBool,
}

/// Connection info the frontend needs to attach the noVNC viewer.
#[derive(Serialize, Clone)]
struct RunningInfo {
    name: String,
    websocket_port: u16,
    vnc_display: u16,
    qmp_port: u16,
    spice_port: u16,
    /// Whether the guest's vCPUs are currently paused (QMP `stop`).
    paused: bool,
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
        .filter_map(|(name, rv)| {
            let alive = match rv.child.as_mut() {
                // We own it: try_wait reaps it and reports whether it exited.
                Some(child) => !matches!(child.try_wait(), Ok(Some(_))),
                // Reattached: no handle, so check the PID directly.
                None => procutil::pid_alive(rv.pid),
            };
            if alive {
                None
            } else {
                Some(name.clone())
            }
        })
        .collect();
    for name in dead {
        running.remove(&name);
    }
}

/// Record the currently-running VMs to disk so a relaunch can reattach to the
/// QEMU processes (which now outlive the app). Best-effort.
fn persist_running(app: &AppHandle, running: &HashMap<String, RunningVm>) {
    if let Ok(dir) = app_data(app) {
        let list: Vec<session::PersistedVm> = running
            .iter()
            .map(|(name, rv)| session::PersistedVm {
                name: name.clone(),
                pid: rv.pid,
                vnc_display: rv.vnc_display,
                websocket_port: rv.websocket_port,
                qmp_port: rv.qmp_port,
                spice_port: rv.spice_port,
            })
            .collect();
        let _ = session::save(&dir, &list);
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
    // Keep the on-disk record current as VMs come and go (also reaps any that
    // exited between polls).
    persist_running(&app, &running);

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

/// Normalize a shared-folder path from the UI: trim, treat empty as "none", and
/// reject commas. The path flows into a `-drive file=fat:...,format=raw,...`
/// option string, whose fields are comma-separated — a comma in the path would
/// be misparsed as a drive option (QEMU's own escape is doubling, which we don't
/// want to expose), so reject it with a clear message instead.
fn clean_shared_folder(path: Option<String>) -> Result<Option<String>, String> {
    let path = match path {
        Some(p) => p.trim().to_string(),
        None => return Ok(None),
    };
    if path.is_empty() {
        return Ok(None);
    }
    if path.contains(',') {
        return Err("Shared-folder path can't contain a comma".into());
    }
    Ok(Some(path))
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
    nic_model: String,
    port_forwards: Vec<vm::PortForward>,
    net_mode: String,
    acceleration: String,
    shared_folder: Option<String>,
    shared_folder_writable: bool,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("VM name cannot be empty".into());
    }
    let shared_folder = clean_shared_folder(shared_folder)?;
    // Reject path-unsafe characters (the name is also a directory name) and the
    // comma, which would break QEMU's `-name` option parsing and our reattach
    // identity check (qmp.rs::query_name).
    if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ',']) {
        return Err("VM name can't contain any of  / \\ : * ? \" < > | ,".into());
    }
    let dir = app_data(&app)?;
    if vm::machine_dir(&dir, &name).exists() {
        return Err(format!("A VM named '{name}' already exists"));
    }

    let disk_path = vm::machine_dir(&dir, &name)
        .join("disk.qcow2")
        .to_string_lossy()
        .into_owned();

    // Generate and persist a stable MAC up front so it shows in the UI and
    // never changes for this VM.
    let mac_address = Some(qemu::derive_mac(&name));

    let cfg = VmConfig {
        name,
        memory_mb,
        cpus,
        disk_path: disk_path.clone(),
        disk_size_gb,
        iso_path: iso_path.filter(|s| !s.is_empty()),
        display_adapter,
        nic_model,
        port_forwards,
        net_mode,
        mac_address,
        acceleration,
        shared_folder,
        shared_folder_writable,
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
    nic_model: String,
    port_forwards: Vec<vm::PortForward>,
    net_mode: String,
    acceleration: String,
    shared_folder: Option<String>,
    shared_folder_writable: bool,
) -> Result<(), String> {
    if memory_mb < 64 {
        return Err("Memory must be at least 64 MB".into());
    }
    if cpus < 1 {
        return Err("Need at least 1 CPU".into());
    }
    let shared_folder = clean_shared_folder(shared_folder)?;
    let dir = app_data(&app)?;
    let mut cfg = vm::load(&dir, &name)?;
    cfg.memory_mb = memory_mb;
    cfg.cpus = cpus;
    cfg.iso_path = iso_path.filter(|s| !s.is_empty());
    cfg.display_adapter = display_adapter;
    cfg.nic_model = nic_model;
    cfg.port_forwards = port_forwards;
    cfg.net_mode = net_mode;
    cfg.acceleration = acceleration;
    cfg.shared_folder = shared_folder;
    cfg.shared_folder_writable = shared_folder_writable;
    // Backfill a stable MAC for VMs created before the field existed.
    if cfg.mac_address.is_none() {
        cfg.mac_address = Some(qemu::derive_mac(&cfg.name));
    }
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
        spice_port: qemu::free_port()?,
    };
    let args = qemu::build_args(&cfg, &ports);

    // QEMU's output goes to a per-VM log file rather than a pipe we own, so the
    // guest keeps running independently after yodawg exits (nothing it writes
    // depends on a handle we hold). The log doubles as a launch-failure record.
    let log_path = vm::machine_dir(&dir, &name).join("qemu.log");
    let log = std::fs::File::create(&log_path)
        .map_err(|e| format!("creating {}: {e}", log_path.display()))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(qemu::system_binary());
    cmd.args(&args).stdout(log).stderr(log_err);
    // Run QEMU from its install dir so it can find data files (e.g. the VNC
    // keymap 'en-us'), which it looks up relative to the working directory.
    if let Some(qdir) = qemu::install_dir() {
        cmd.current_dir(qdir);
    }
    // Don't pop a console window for the console-mode qemu-system exe when
    // launched from the GUI app (its output already goes to qemu.log).
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch QEMU: {e}"))?;
    let pid = child.id();

    // Bad args / missing acceleration usually make QEMU exit within a moment.
    // Give it a beat, and if it died, surface the log instead of silently
    // flipping the VM back to "stopped".
    std::thread::sleep(Duration::from_millis(600));
    if let Ok(Some(status)) = child.try_wait() {
        let msg = std::fs::read_to_string(&log_path).unwrap_or_default();
        let msg = msg.trim();
        return Err(if msg.is_empty() {
            format!("QEMU exited immediately ({status}).")
        } else {
            format!("QEMU failed to start ({status}):\n{msg}")
        });
    }

    let info = RunningInfo {
        name: name.clone(),
        websocket_port: ports.websocket_port,
        vnc_display: ports.vnc_display,
        qmp_port: ports.qmp_port,
        spice_port: ports.spice_port,
        paused: false, // freshly launched -> running
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
                child: Some(child),
                pid,
                vnc_display: ports.vnc_display,
                websocket_port: ports.websocket_port,
                qmp_port: ports.qmp_port,
                spice_port: ports.spice_port,
            },
        );
        persist_running(&app, &running);
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
        spice_port: rv.spice_port,
        // QMP may not be ready yet during early boot; treat errors as "running".
        paused: qmp::is_paused(rv.qmp_port).unwrap_or(false),
    })
}

/// Launch the external virt-viewer (`remote-viewer`) SPICE client against a
/// running VM. The embedded noVNC viewer keeps working independently — this is
/// the richer client (clipboard, dynamic resolution, USB redirection). The
/// spawned viewer is a detached, untracked GUI window; we don't manage its
/// lifecycle (closing it just disconnects, the guest keeps running).
#[tauri::command]
fn open_in_viewer(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let spice_port = {
        let mut running = state.running.lock().unwrap();
        prune_dead(&mut running);
        running
            .get(&name)
            .map(|rv| rv.spice_port)
            .ok_or_else(|| format!("'{name}' is not running"))?
    };
    // A spice_port of 0 means this VM was reattached from a pre-SPICE session
    // (its QEMU has no SPICE server). Restart the VM to get a real one.
    if spice_port == 0 {
        return Err(
            "This VM was started before SPICE support — stop and start it again to \
open it in virt-viewer."
                .into(),
        );
    }
    let viewer = qemu::viewer_binary().ok_or_else(|| {
        "virt-viewer not found. Install it (the bundled yodawg installer adds it), or set \
the YODAWG_VIRT_VIEWER environment variable to the full path of remote-viewer.exe."
            .to_string()
    })?;
    let mut cmd = std::process::Command::new(viewer);
    cmd.arg(format!("spice://127.0.0.1:{spice_port}"));
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch virt-viewer: {e}"))
}

/// Gracefully shut down a VM via QMP (ACPI power button). The guest OS decides
/// when to actually power off; the process exits on its own and `prune_dead`
/// reaps it.
#[tauri::command]
fn stop_vm(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let qmp_port = running_qmp_port(&state, &name)?;
    // A paused guest has halted vCPUs and can't act on the ACPI power button
    // until it's running again, so resume first (`cont` is a no-op if it's
    // already running), then request the powerdown.
    let _ = qmp::execute(qmp_port, "cont");
    qmp::execute(qmp_port, "system_powerdown")?;
    Ok(())
}

/// Pause a running VM (QMP `stop` — halts vCPUs). Works on any accelerator.
#[tauri::command]
fn pause_vm(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let qmp_port = running_qmp_port(&state, &name)?;
    qmp::execute(qmp_port, "stop")?;
    Ok(())
}

/// Resume a paused VM (QMP `cont`).
#[tauri::command]
fn resume_vm(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let qmp_port = running_qmp_port(&state, &name)?;
    qmp::execute(qmp_port, "cont")?;
    Ok(())
}

/// Forcibly terminate a VM's process (the "pull the plug" option).
#[tauri::command]
fn force_kill_vm(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let mut running = state.running.lock().unwrap();
    let mut rv = running
        .remove(&name)
        .ok_or_else(|| format!("'{name}' is not running"))?;
    match rv.child.as_mut() {
        Some(child) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        None => procutil::kill_pid(rv.pid), // reattached: terminate by PID
    }
    persist_running(&app, &running);
    Ok(())
}

/// Validate a snapshot tag. Tags flow into an HMP command line (`savevm <tag>`),
/// so whitespace and shell-ish characters are rejected to keep them parseable.
fn validate_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Err("Snapshot name cannot be empty".into());
    }
    if tag.chars().any(|c| c.is_whitespace())
        || tag.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\''])
    {
        return Err("Snapshot name contains invalid characters".into());
    }
    Ok(())
}

/// The QMP port for a VM if it is currently running, else `None`.
fn qmp_port_if_running(state: &AppState, name: &str) -> Option<u16> {
    let mut running = state.running.lock().unwrap();
    prune_dead(&mut running);
    running.get(name).map(|rv| rv.qmp_port)
}

/// The QMP port for a running VM, or an error if it isn't running. (The lock is
/// released before the caller talks to QMP.)
fn running_qmp_port(state: &AppState, name: &str) -> Result<u16, String> {
    qmp_port_if_running(state, name).ok_or_else(|| format!("'{name}' is not running"))
}

/// Message shown when a live snapshot op is attempted on an accelerator that
/// can't migrate VM state (WHPX/HVF).
const LIVE_UNSUPPORTED: &str =
    "Live snapshots aren't supported by this VM's accelerator (WHPX on Windows / \
HVF on macOS). Shut the VM down first, then snapshot its disk.";

/// Whether snapshots can be taken/restored while this VM is running. Depends on
/// the VM's accelerator (TCG/KVM support it; WHPX/HVF don't).
#[tauri::command]
fn live_snapshots_supported(app: AppHandle, name: String) -> Result<bool, String> {
    let dir = app_data(&app)?;
    let cfg = vm::load(&dir, &name)?;
    Ok(qemu::live_snapshots_supported(&cfg))
}

/// List the snapshots stored in a VM's disk. Works whether or not it's running.
#[tauri::command]
fn list_snapshots(app: AppHandle, name: String) -> Result<Vec<qemu::Snapshot>, String> {
    let dir = app_data(&app)?;
    let cfg = vm::load(&dir, &name)?;
    qemu::list_snapshots(&cfg.disk_path)
}

/// Create a snapshot. A running VM is snapshotted live via QMP (`savevm`, which
/// also saves RAM); a stopped VM is snapshotted on-disk via `qemu-img`.
#[tauri::command]
fn create_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tag: String,
) -> Result<(), String> {
    validate_tag(&tag)?;
    let dir = app_data(&app)?;
    let cfg = vm::load(&dir, &name)?;
    match qmp_port_if_running(&state, &name) {
        Some(_) if !qemu::live_snapshots_supported(&cfg) => Err(LIVE_UNSUPPORTED.into()),
        Some(port) => qmp::hmp(port, &format!("savevm {tag}")),
        None => qemu::snapshot_create_offline(&cfg.disk_path, &tag),
    }
}

/// Restore a snapshot, discarding the VM's current state. A running VM restores
/// live via QMP (`loadvm`); a stopped VM restores on-disk via `qemu-img`.
#[tauri::command]
fn restore_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tag: String,
) -> Result<(), String> {
    let dir = app_data(&app)?;
    let cfg = vm::load(&dir, &name)?;
    match qmp_port_if_running(&state, &name) {
        Some(_) if !qemu::live_snapshots_supported(&cfg) => Err(LIVE_UNSUPPORTED.into()),
        Some(port) => qmp::hmp(port, &format!("loadvm {tag}")),
        None => qemu::snapshot_apply_offline(&cfg.disk_path, &tag),
    }
}

/// Delete a snapshot. Routed to QMP (`delvm`) when running, `qemu-img` when not.
#[tauri::command]
fn delete_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tag: String,
) -> Result<(), String> {
    match qmp_port_if_running(&state, &name) {
        Some(port) => qmp::hmp(port, &format!("delvm {tag}")),
        None => {
            let dir = app_data(&app)?;
            let cfg = vm::load(&dir, &name)?;
            qemu::snapshot_delete_offline(&cfg.disk_path, &tag)
        }
    }
}

/// Pause every running guest (QMP `stop`) on app close and record them, so the
/// VMs stay suspended in the background — consuming no CPU — for a later
/// relaunch to reattach and resume, rather than being powered off or killed.
fn pause_all_on_exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let running = state.running.lock().unwrap();
    for rv in running.values() {
        let _ = qmp::execute(rv.qmp_port, "stop");
    }
    persist_running(app, &running);
}

/// On launch, reattach to any VMs a previous session left running/paused. A
/// persisted entry is trusted only if its PID is alive *and* QMP reports the
/// matching guest name (set via `-name`), so a reused PID or port can't be
/// mistaken for one of our VMs. Survivors are repopulated; the rest are dropped.
fn reconcile_running(app: &AppHandle) {
    let dir = match app_data(app) {
        Ok(d) => d,
        Err(_) => return,
    };
    let state = app.state::<AppState>();
    let mut running = state.running.lock().unwrap();
    let mut survivors: Vec<session::PersistedVm> = Vec::new();
    for p in session::load(&dir) {
        if !procutil::pid_alive(p.pid) {
            continue;
        }
        if qmp::query_name(p.qmp_port).ok().flatten().as_deref() != Some(p.name.as_str()) {
            continue; // not our VM (gone, or the port belongs to something else)
        }
        running.insert(
            p.name.clone(),
            RunningVm {
                child: None,
                pid: p.pid,
                vnc_display: p.vnc_display,
                websocket_port: p.websocket_port,
                qmp_port: p.qmp_port,
                spice_port: p.spice_port,
            },
        );
        survivors.push(p);
    }
    let _ = session::save(&dir, &survivors);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState {
        running: Mutex::new(HashMap::new()),
        shutting_down: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Reattach to any VMs a previous session left paused in the
            // background, so they show up running and can be resumed.
            //
            // Done on a background thread so it never blocks the event loop:
            // reconcile makes a blocking QMP TCP round-trip per persisted VM
            // (each up to a 5s timeout), and `setup` runs before the window
            // paints — doing it inline stalls the whole UI on launch. The
            // frontend polls `list_vms` every few seconds, so reattached VMs
            // flip to "running" as soon as this finishes (the `running` mutex
            // keeps it race-safe against concurrent commands).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                reconcile_running(&handle);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // When the user closes the window with VMs running, hold the close,
            // pause the guests on a background thread (leaving them alive for a
            // later relaunch to reattach), then exit.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let state = app.state::<AppState>();

                let any_running = {
                    let mut running = state.running.lock().unwrap();
                    prune_dead(&mut running);
                    !running.is_empty()
                };
                if !any_running {
                    return; // nothing to pause; let it close
                }
                if state.shutting_down.swap(true, Ordering::SeqCst) {
                    return; // close sequence already underway
                }

                api.prevent_close();
                let app2 = app.clone();
                std::thread::spawn(move || {
                    pause_all_on_exit(&app2);
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
            open_in_viewer,
            stop_vm,
            pause_vm,
            resume_vm,
            force_kill_vm,
            live_snapshots_supported,
            list_snapshots,
            create_snapshot,
            restore_snapshot,
            delete_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
