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

/// The accelerator to actually launch this VM with. `"tcg"` (set per-VM) forces
/// pure software emulation; anything else uses the platform's hardware
/// accelerator. TCG is slower for CPU-bound guests but avoids the VM-exit storm
/// that hardware hypervisors hit on direct planar-VGA writes (DOS games).
fn effective_accel(cfg: &VmConfig) -> &'static str {
    if cfg.acceleration == "tcg" {
        "tcg"
    } else {
        accelerator()
    }
}

/// Whether snapshots can be taken/restored while *this* VM is running.
///
/// `savevm`/`loadvm` save and restore migratable vCPU + RAM state. KVM and TCG
/// provide that; WHPX (Windows) and HVF (macOS) do not — QEMU rejects the save
/// with "State blocked due to non-migratable CPUID/dirty-memory/XSAVE support".
/// So under those accelerators snapshots are only available while the VM is
/// stopped (disk-only, via `qemu-img`). A TCG VM supports live snapshots on any
/// host.
pub fn live_snapshots_supported(cfg: &VmConfig) -> bool {
    matches!(effective_accel(cfg), "tcg" | "kvm")
}

/// A CPU model that is safe for the chosen accelerator.
///
/// `-cpu max`/`-cpu host` crash WHPX due to feature conflicts (APX vs MPX), so
/// on Windows we pin a conservative model. KVM/HVF can use the host CPU. TCG
/// can't passthrough the host CPU at all (`-cpu host` is invalid under software
/// emulation), so a TCG VM always gets the conservative `qemu64`.
fn cpu_model(cfg: &VmConfig) -> &'static str {
    if effective_accel(cfg) == "tcg" || cfg!(target_os = "windows") {
        "qemu64"
    } else {
        "host"
    }
}

/// A deterministic MAC address for a VM, derived from its name.
///
/// Uses QEMU's standard `52:54:00` OUI for the first three octets and a hash of
/// the name for the last three, so the same VM always gets the same MAC (stable
/// DHCP lease) while different VMs almost never collide. Generated once at
/// create time and persisted; this is the fallback for configs that predate the
/// stored `mac_address` field.
pub fn derive_mac(name: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    let n = h.finish();
    format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        (n >> 16) as u8,
        (n >> 8) as u8,
        n as u8,
    )
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
    /// SPICE server port for the external virt-viewer client (separate from the
    /// VNC/noVNC path, which keeps using `vnc_display`/`websocket_port`).
    pub spice_port: u16,
}

/// Full path to virt-viewer's `remote-viewer`, or `None` if it can't be found.
///
/// On Windows, the order is: the `YODAWG_VIRT_VIEWER` env override (a full path
/// to the exe), then a scan of the Program Files dirs for a
/// `VirtViewer*\bin\remote-viewer.exe`. The installer creates a version-stamped
/// directory (e.g. `VirtViewer v11.0-256`), so we glob the prefix rather than
/// hardcode a version. virt-viewer doesn't add itself to PATH, so there's no
/// bare-name fallback — `None` lets the caller surface an install hint. On other
/// platforms `remote-viewer` is normally on PATH, so return the bare name.
#[cfg(target_os = "windows")]
pub fn viewer_binary() -> Option<String> {
    if let Ok(p) = std::env::var("YODAWG_VIRT_VIEWER") {
        if PathBuf::from(&p).exists() {
            return Some(p);
        }
    }
    let roots = [
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into()),
        std::env::var("ProgramFiles(x86)")
            .unwrap_or_else(|_| r"C:\Program Files (x86)".into()),
    ];
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("VirtViewer") {
                    let exe = entry.path().join("bin").join("remote-viewer.exe");
                    if exe.exists() {
                        return Some(exe.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
pub fn viewer_binary() -> Option<String> {
    Some("remote-viewer".into())
}

/// Build the full QEMU argument vector for a VM.
///
/// Display strategy: `-display none` (no QEMU window) plus `-vnc` with a
/// `websocket=` listener, which the embedded noVNC client connects to directly.
pub fn build_args(cfg: &VmConfig, ports: &LaunchPorts) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-accel".into(),
        effective_accel(cfg).into(),
        "-cpu".into(),
        cpu_model(cfg).into(),
        "-m".into(),
        cfg.memory_mb.to_string(),
        "-smp".into(),
        cfg.cpus.to_string(),
        // Primary disk. The `file=` prefix is mandatory — omitting it yields
        // cryptic errors.
        //
        // `cache=writethrough`: commit every guest write to the qcow2 file
        // immediately instead of QEMU's default `writeback` (which buffers
        // writes in QEMU's own memory until a guest flush). Non-ACPI guests
        // (DOS) ignore our `system_powerdown` "Shut down", so the only way to
        // stop them is Force kill — a hard process terminate. Under writeback
        // that drops unflushed writes and corrupts qcow2 metadata, so a freshly
        // partitioned/formatted disk comes back blank on the next boot.
        // Writethrough makes a hard kill safe at the cost of some write speed.
        "-drive".into(),
        format!("file={},format=qcow2,cache=writethrough", cfg.disk_path),
        // Tag the guest with its name so we can identify it over QMP
        // (`query-name`) when reattaching to a VM left running across an app
        // restart — see lib.rs::reconcile_running.
        "-name".into(),
        cfg.name.clone(),
    ];

    // Shared folder: expose a host directory to the guest as a virtual FAT disk
    // (vvfat). `file=fat:<dir>` is read-only; `file=fat:rw:<dir>` lets the guest
    // write back to the host folder — convenient but fragile (vvfat's rw path is
    // known to corrupt the directory under heavy/concurrent writes), so it's
    // opt-in via `shared_folder_writable`. `format=raw` skips image-format
    // probing (vvfat isn't a real file), and an explicit `index=1` (IDE primary
    // slave) keeps it clear of the boot disk (index 0) and the `-cdrom` (index
    // 2) so it never displaces the boot device.
    if let Some(dir) = &cfg.shared_folder {
        if !dir.is_empty() {
            let rw = if cfg.shared_folder_writable { "rw:" } else { "" };
            args.push("-drive".into());
            args.push(format!("file=fat:{rw}{dir},format=raw,index=1"));
        }
    }

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

    // Networking. "none" means no NIC at all (QEMU still adds an implicit
    // default NIC unless we suppress it, so pass `-nic none`). "nat" and
    // "isolated" both use the user-mode (NAT) backend; "isolated" adds
    // `restrict=on`, which gives the guest a DHCP lease but blocks it from
    // reaching the host or the internet. Specifying a netdev/device suppresses
    // QEMU's implicit default NIC.
    if cfg.net_mode == "none" {
        args.push("-nic".into());
        args.push("none".into());
    } else {
        let mut netdev = String::from("user,id=net0");
        if cfg.net_mode == "isolated" {
            netdev.push_str(",restrict=on");
        }
        // Port forwards (host:hostPort -> guest:guestPort) are appended as
        // `hostfwd` rules. They keep working under `restrict=on` — restrict
        // only blocks guest-initiated traffic, not host->guest redirection.
        for pf in &cfg.port_forwards {
            let proto = if pf.protocol == "udp" { "udp" } else { "tcp" };
            netdev.push_str(&format!(
                ",hostfwd={proto}::{}-:{}",
                pf.host_port, pf.guest_port
            ));
        }
        args.push("-netdev".into());
        args.push(netdev);

        // A stable MAC keeps the guest's DHCP lease and any MAC-bound licenses
        // constant across reboots. Old configs predate the field, so derive a
        // deterministic one from the VM name when it's absent.
        let mac = cfg
            .mac_address
            .clone()
            .unwrap_or_else(|| derive_mac(&cfg.name));

        // NIC model. e1000 suits modern guests (Linux/Windows have drivers),
        // but DOS-era guests only ship NE2000 packet drivers — those probe an
        // ISA card at I/O 0x300 / IRQ 9, which `ne2k_isa` provides by default,
        // so the driver finds it without reconfiguration. Wrong model => the
        // guest's driver finds no card and reports an all-FF MAC with no lease.
        let model = match cfg.nic_model.as_str() {
            "ne2k" => "ne2k_isa",
            "rtl8139" => "rtl8139",
            "virtio" => "virtio-net-pci",
            _ => "e1000", // default / "e1000"
        };
        args.push("-device".into());
        args.push(format!("{model},netdev=net0,mac={mac}"));
    }

    // VGA model: "std" (broad compatibility), "virtio" (faster for Linux), or
    // "qxl" (the SPICE-native adapter — gives the virt-viewer client dynamic
    // resolution / auto-resize and smoother updates; `-vga qxl` maps to the
    // qxl-vga primary device). qxl still renders fine over VNC too.
    let vga = match cfg.display_adapter.as_str() {
        "virtio" => "virtio",
        "qxl" => "qxl",
        _ => "std",
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

    // SPICE server, running *alongside* the VNC server above (QEMU serves both
    // from the same console). The embedded noVNC viewer keeps using VNC; SPICE
    // is for the external virt-viewer (remote-viewer) client, which adds
    // clipboard sharing, dynamic resolution, and USB redirection that the VNC
    // path can't do. We only listen on loopback, so `disable-ticketing=on`
    // skips password auth. The vdagent channel (virtio-serial + a `vdagent`
    // spicevmc chardev) carries guest-agent traffic for clipboard/auto-resize
    // when the guest has spice-vdagent installed — harmless when it doesn't.
    args.push("-spice".into());
    args.push(format!(
        "port={},addr=127.0.0.1,disable-ticketing=on",
        ports.spice_port
    ));
    args.push("-device".into());
    args.push("virtio-serial-pci".into());
    args.push("-chardev".into());
    args.push("spicevmc,id=spicechannel0,name=vdagent".into());
    args.push("-device".into());
    args.push("virtserialport,chardev=spicechannel0,name=com.redhat.spice.0".into());

    // QMP control channel for lifecycle commands.
    args.push("-qmp".into());
    args.push(format!(
        "tcp:127.0.0.1:{},server,nowait",
        ports.qmp_port
    ));

    args
}
