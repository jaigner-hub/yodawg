//! QEMU binary discovery, disk creation, and command-line construction.
//!
//! All the fiddly knowledge from manual QEMU use lives here so the rest of the
//! app never has to think about flags:
//!   * acceleration backend per platform (WHPX on Windows, KVM on Linux, HVF on macOS)
//!   * a CPU model that won't crash the chosen accelerator (`-cpu max` crashes WHPX)
//!   * the mandatory `file=` prefix on `-drive`
//!   * VNC + websocket wiring for the embedded noVNC viewer
//!   * a QMP control socket for clean lifecycle management

use crate::vm::VmConfig;
use serde::Serialize;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

/// Suppress the console window when spawning a console-mode QEMU exe.
///
/// The installed app is a GUI-subsystem process with no inherited console, so
/// each `qemu`/`qemu-img` spawn would otherwise pop a brief command-prompt
/// window — and the console alloc/teardown also stalls startup when `detect()`
/// runs the `--version` check. (`tauri dev` is console-subsystem, so the child
/// inherits its console and this is a no-op there.)
fn no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = cmd;
}

/// Resolve the directory containing the QEMU binaries.
///
/// Order: `YODAWG_QEMU_DIR` env override, then the default install location,
/// then fall back to bare names (relying on PATH).
fn qemu_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("YODAWG_QEMU_DIR") {
        let p = PathBuf::from(dir);
        if p.exists() {
            return Some(p);
        }
    }
    #[cfg(target_os = "windows")]
    {
        let default = PathBuf::from(r"C:\Program Files\qemu");
        if default.exists() {
            return Some(default);
        }
    }
    None
}

#[cfg(target_os = "windows")]
const SYSTEM_BIN: &str = "qemu-system-x86_64.exe";
#[cfg(not(target_os = "windows"))]
const SYSTEM_BIN: &str = "qemu-system-x86_64";

#[cfg(target_os = "windows")]
const IMG_BIN: &str = "qemu-img.exe";
#[cfg(not(target_os = "windows"))]
const IMG_BIN: &str = "qemu-img";

/// Full path to `qemu-system-*`, or the bare name if no install dir is known.
pub fn system_binary() -> String {
    match qemu_dir() {
        Some(dir) => dir.join(SYSTEM_BIN).to_string_lossy().into_owned(),
        None => SYSTEM_BIN.to_string(),
    }
}

/// The QEMU install directory, if known. QEMU is launched with this as its
/// working directory: it resolves some data files (notably the VNC keymap
/// `en-us`) relative to the CWD, which otherwise fails in the installed GUI
/// build (launched from a system dir) — see `lib.rs::start_vm`.
pub fn install_dir() -> Option<PathBuf> {
    qemu_dir()
}

/// Full path to `qemu-img`, or the bare name if no install dir is known.
pub fn img_binary() -> String {
    match qemu_dir() {
        Some(dir) => dir.join(IMG_BIN).to_string_lossy().into_owned(),
        None => IMG_BIN.to_string(),
    }
}

/// Report whether QEMU was found, and its version string. Used for a startup
/// health check surfaced in the UI.
pub fn detect() -> Result<String, String> {
    let mut cmd = Command::new(system_binary());
    cmd.arg("--version");
    no_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("Could not run QEMU ({}): {e}", system_binary()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or("unknown").trim().to_string())
}

/// The acceleration flag value for the current platform.
fn accelerator() -> &'static str {
    if cfg!(target_os = "windows") {
        "whpx"
    } else if cfg!(target_os = "macos") {
        "hvf"
    } else {
        "kvm"
    }
}

/// Whether the current accelerator supports live (running-VM) snapshots.
///
/// `savevm`/`loadvm` save and restore migratable vCPU + RAM state. KVM provides
/// that; WHPX (Windows) and HVF (macOS) do not — QEMU rejects the save with
/// "State blocked due to non-migratable CPUID/dirty-memory/XSAVE support". So on
/// those platforms snapshots are only available while the VM is stopped
/// (disk-only, via `qemu-img`).
pub fn live_snapshots_supported() -> bool {
    cfg!(target_os = "linux")
}

/// A CPU model that is safe for the chosen accelerator.
///
/// `-cpu max`/`-cpu host` crash WHPX due to feature conflicts (APX vs MPX), so
/// on Windows we pin a conservative model. KVM/HVF can use the host CPU.
fn cpu_model() -> &'static str {
    if cfg!(target_os = "windows") {
        "qemu64"
    } else {
        "host"
    }
}

/// Create a qcow2 disk image of the given size (in GiB).
pub fn create_disk(path: &str, size_gb: u32) -> Result<(), String> {
    let mut cmd = Command::new(img_binary());
    cmd.args(["create", "-f", "qcow2", path, &format!("{size_gb}G")]);
    no_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("Could not run qemu-img: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "qemu-img create failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// A qcow2 internal snapshot, as reported by `qemu-img`.
#[derive(Serialize, Clone, Debug)]
pub struct Snapshot {
    /// The snapshot tag (qcow2 calls this the snapshot "name").
    pub tag: String,
    /// Saved machine-state size in bytes. `0` means a disk-only snapshot
    /// (taken while the VM was stopped); non-zero means RAM was captured too.
    pub vm_state_size: u64,
    /// Wall-clock time the snapshot was taken, as a Unix timestamp (seconds).
    pub date_sec: i64,
}

/// Run `qemu-img` with the given args, returning an error with stderr on failure.
fn run_img(args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new(img_binary());
    cmd.args(args);
    no_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("Could not run qemu-img: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "qemu-img {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out)
}

/// List the internal snapshots stored in a qcow2 disk.
///
/// Reads `qemu-img info --output=json`, whose `snapshots` array is the source of
/// truth regardless of whether the VM is running — so the same call serves both
/// states without scraping QMP output.
pub fn list_snapshots(disk_path: &str) -> Result<Vec<Snapshot>, String> {
    let out = run_img(&["info", "--output=json", disk_path])?;
    let info: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("parsing qemu-img info: {e}"))?;
    let snaps = match info.get("snapshots").and_then(|s| s.as_array()) {
        Some(arr) => arr,
        None => return Ok(Vec::new()), // no snapshots key -> none taken yet
    };
    let list = snaps
        .iter()
        .map(|s| Snapshot {
            tag: s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            vm_state_size: s.get("vm-state-size").and_then(|v| v.as_u64()).unwrap_or(0),
            date_sec: s.get("date-sec").and_then(|v| v.as_i64()).unwrap_or(0),
        })
        .collect();
    Ok(list)
}

/// Create a disk-only snapshot (for a stopped VM). Unsafe to call while QEMU has
/// the disk open — running VMs snapshot via QMP instead.
pub fn snapshot_create_offline(disk_path: &str, tag: &str) -> Result<(), String> {
    run_img(&["snapshot", "-c", tag, disk_path]).map(|_| ())
}

/// Apply (restore) a snapshot into a stopped VM's disk.
pub fn snapshot_apply_offline(disk_path: &str, tag: &str) -> Result<(), String> {
    run_img(&["snapshot", "-a", tag, disk_path]).map(|_| ())
}

/// Delete a snapshot from a stopped VM's disk.
pub fn snapshot_delete_offline(disk_path: &str, tag: &str) -> Result<(), String> {
    run_img(&["snapshot", "-d", tag, disk_path]).map(|_| ())
}

/// Ask the OS for a free TCP port by binding to port 0 and reading it back.
/// There is a small race between releasing this and QEMU binding it, but it is
/// the standard approach and fine for local single-user use.
pub fn free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("no free port: {e}"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

/// Find a free VNC display number `D` (so the RFB port 5900+D is unused).
pub fn free_vnc_display() -> Result<u16, String> {
    for d in 0u16..100 {
        if TcpListener::bind(("127.0.0.1", 5900 + d)).is_ok() {
            return Ok(d);
        }
    }
    Err("no free VNC display in 0..100".into())
}

/// Ports/handles assigned to a launched VM.
pub struct LaunchPorts {
    pub vnc_display: u16,
    pub websocket_port: u16,
    pub qmp_port: u16,
}

/// Build the full QEMU argument vector for a VM.
///
/// Display strategy: `-display none` (no QEMU window) plus `-vnc` with a
/// `websocket=` listener, which the embedded noVNC client connects to directly.
pub fn build_args(cfg: &VmConfig, ports: &LaunchPorts) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-accel".into(),
        accelerator().into(),
        "-cpu".into(),
        cpu_model().into(),
        "-m".into(),
        cfg.memory_mb.to_string(),
        "-smp".into(),
        cfg.cpus.to_string(),
        // Primary disk. The `file=` prefix is mandatory — omitting it yields
        // cryptic errors.
        "-drive".into(),
        format!("file={},format=qcow2", cfg.disk_path),
        // Tag the guest with its name so we can identify it over QMP
        // (`query-name`) when reattaching to a VM left running across an app
        // restart — see lib.rs::reconcile_running.
        "-name".into(),
        cfg.name.clone(),
    ];

    // Attach the install ISO when present. Boot order is hard-disk-first with
    // CD-ROM fallback (`order=cd`: c=disk, d=cdrom): a blank disk falls through
    // to the installer, but once the OS is installed the disk boots itself —
    // so the ISO can stay attached without trapping you on the installer.
    //
    // `menu=on` enables SeaBIOS's interactive boot menu (press Esc at boot) so
    // you can force a CD boot when the disk-first order gets in the way. This
    // matters for multi-stage installers that reboot mid-install and expect to
    // land back on the CD (FreeDOS partitions, reboots, then formats from the
    // CD): after partitioning, the half-baked MBR halts with "Invalid partition
    // table" instead of falling through to the CD, so without a menu you can't
    // reach the installer's second stage. `splash-time` (ms) widens the prompt
    // window so it's catchable over VNC, where early keypresses can lag.
    if let Some(iso) = &cfg.iso_path {
        if !iso.is_empty() {
            args.push("-cdrom".into());
            args.push(iso.clone());
            args.push("-boot".into());
            args.push("order=cd,menu=on,splash-time=4000".into());
        }
    }

    // Absolute pointing device. Without this, QEMU defaults to a relative PS/2
    // mouse, so the guest cursor drifts away from the host cursor over VNC (the
    // pointer appears offset and "escapes" the display). A USB tablet reports
    // absolute coordinates, keeping the two cursors aligned 1:1.
    args.push("-usb".into());
    args.push("-device".into());
    args.push("usb-tablet".into());

    // User-mode (NAT) networking. Specifying a netdev/device suppresses QEMU's
    // implicit default NIC. Any configured port forwards are appended as
    // `hostfwd` rules (host:hostPort -> guest:guestPort).
    let mut netdev = String::from("user,id=net0");
    for pf in &cfg.port_forwards {
        let proto = if pf.protocol == "udp" { "udp" } else { "tcp" };
        netdev.push_str(&format!(
            ",hostfwd={proto}::{}-:{}",
            pf.host_port, pf.guest_port
        ));
    }
    args.push("-netdev".into());
    args.push(netdev);
    // NIC model. e1000 suits modern guests (Linux/Windows have drivers), but
    // DOS-era guests only ship NE2000 packet drivers — those probe an ISA card
    // at I/O 0x300 / IRQ 9, which `ne2k_isa` provides by default, so the driver
    // finds it without reconfiguration. Wrong model => the guest's driver finds
    // no card and reports an all-FF MAC with no DHCP lease.
    let nic_device = match cfg.nic_model.as_str() {
        "ne2k" => "ne2k_isa,netdev=net0",
        "rtl8139" => "rtl8139,netdev=net0",
        "virtio" => "virtio-net-pci,netdev=net0",
        _ => "e1000,netdev=net0", // default / "e1000"
    };
    args.push("-device".into());
    args.push(nic_device.into());

    // VGA model: "std" (broad compatibility) or "virtio" (faster for Linux).
    let vga = if cfg.display_adapter == "virtio" {
        "virtio"
    } else {
        "std"
    };
    args.push("-vga".into());
    args.push(vga.into());

    // No native window; render over VNC with a websocket listener for noVNC.
    args.push("-display".into());
    args.push("none".into());
    args.push("-vnc".into());
    args.push(format!(
        "127.0.0.1:{},websocket={}",
        ports.vnc_display, ports.websocket_port
    ));

    // QMP control channel for lifecycle commands.
    args.push("-qmp".into());
    args.push(format!(
        "tcp:127.0.0.1:{},server,nowait",
        ports.qmp_port
    ));

    args
}
