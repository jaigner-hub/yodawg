# yodawg

A friendly, cross-platform GUI for [QEMU](https://www.qemu.org/) — a VirtualBox-like
experience with QEMU/KVM performance underneath.

QEMU is fast and powerful but its command line is intimidating and there's no
great desktop UI for it. yodawg wraps QEMU so normal people can create, run, and
manage virtual machines without touching a flag, while still getting native
hardware acceleration (WHPX on Windows, and KVM/HVF later).

![yodawg running MS-DOS / Windows 3.1 with the Program Manager open](docs/screenshot.png)

> Status: **v0.2.10**. Primary target today is **Windows native** (WHPX).
> Working codename, subject to change.

## Features

- **Create-VM wizard** — pick an ISO, set RAM / CPU / disk size, choose a
  display and network adapter, and it creates the `qcow2` disk and boots.
- **VM list** with live running / paused / stopped status.
- **Embedded display** — the VM renders right inside the app window (QEMU VNC
  websocket + [noVNC](https://novnc.com/)), with a **Fit ⇄ 1:1** toggle. 1:1
  fixes mouse drift on guests that only have a relative pointer (DOS, Windows 3.1).
- **SPICE via virt-viewer** — every VM also runs a SPICE server alongside the
  embedded display. **Open in virt-viewer** launches the external
  [virt-viewer](https://virt-manager.org/) client for clipboard sharing,
  dynamic display resize, and USB redirection. Pick the **QXL** or **VirtIO**
  display adapter for the best experience; the guest needs `spice-vdagent` for
  clipboard/auto-resize. virt-viewer is bundled with the Windows installer.
- **Lifecycle controls** — start, graceful ACPI shutdown, pause / resume, force
  kill (disk writes are flushed first so nothing is lost), and delete.
- **Snapshots** — save and restore full VM state. Snapshots can be taken live on
  a running guest where QEMU supports it.
- **Networking** — pick a mode (**NAT** with internet access, **Isolated** —
  the guest gets a DHCP lease but can't reach the host or internet, or **None**),
  set up host→guest **port forwarding**, and choose the NIC model (Intel e1000,
  VirtIO, RTL8139, NE2000 for DOS). Each VM keeps a **stable MAC** so DHCP leases
  and MAC-bound licenses survive reboots.
- **Shared folder** — share a host folder into the guest as a virtual FAT disk
  (QEMU vvfat) to move files in without networking. Read-only by default; an
  optional (experimental) writable mode lets the guest write back. See
  [Moving files in and out of a VM](#moving-files-in-and-out-of-a-vm).
- **Edit settings** — change RAM, CPUs, display/network adapter, networking mode,
  port forwards, the shared folder, and the attached ISO of a stopped VM; eject
  the ISO.
- **Just works defaults** — acceleration and a safe CPU model are auto-selected
  per platform; absolute-pointer mouse (USB tablet) so the cursor tracks 1:1;
  disk-first boot order so installed systems boot themselves.
- **Pause on exit, resume on reopen** — closing the window suspends running
  guests (they keep their state in the background); reopening yodawg reattaches
  to them so you can pick up where you left off.

## Requirements

To **run** the app:

- Windows 10/11 with the **Windows Hypervisor Platform** feature enabled
  (for WHPX acceleration), and the **WebView2** runtime (preinstalled on current Windows).
- **QEMU for Windows** installed (default lookup: `C:\Program Files\qemu`). Set the
  `YODAWG_QEMU_DIR` environment variable to point at a custom install.

To **build** it:

- [Rust](https://rustup.rs/) (MSVC toolchain) + the Visual Studio C++ build tools
- [Node.js](https://nodejs.org/) 18+
- The Tauri prerequisites for your OS: https://tauri.app/start/prerequisites/

## Development

```bash
npm install            # install frontend deps
npm run tauri dev      # run the app with hot reload
npm run tauri build    # produce a release bundle
```

Useful sub-commands:

```bash
npm run build                       # typecheck + bundle the frontend only
cargo check --manifest-path src-tauri/Cargo.toml   # check the Rust backend
```

> **Building from WSL?** The toolchain must run against the Windows filesystem
> and Windows binaries. See [CLAUDE.md](./CLAUDE.md) for the interop workflow and
> gotchas.

### Installer (Windows, bundles QEMU)

`npm run tauri build` produces an NSIS installer at
`src-tauri/target/release/bundle/nsis/yodawg_<version>_x64-setup.exe`. It
bundles both the **QEMU** setup and the **virt-viewer** (SPICE client)
installer, and during install checks whether each is already present — running
the bundled installer silently only if it's missing (QEMU at
`C:\Program Files\qemu`; virt-viewer in any `C:\Program Files\VirtViewer*`).
Because the bundled dependencies install into Program Files, the installer is
**per-machine** (requires admin / one UAC prompt).

Before building, drop the two bundled installers into `src-tauri/installer/`
(both gitignored — large — and embedded by the NSIS hook in
`src-tauri/installer/hooks.nsh`):

- `qemu-w64-setup.exe` — the QEMU for Windows setup (~200 MB)
- `virt-viewer.msi` — the virt-viewer MSI (~80 MB), from
  [virt-manager.org/download](https://virt-manager.org/download/)

## How it works

- **Frontend** (`src/`) — React + TypeScript in a Tauri webview. Renders the VM
  list, the create/edit dialogs, and the embedded noVNC viewer.
- **Backend** (`src-tauri/src/`) — Rust. Spawns and tracks `qemu-system-x86_64`
  processes, controls them over QMP, and persists VM configs.
  - `qemu.rs` — binary discovery, acceleration/CPU selection, QEMU argument
    building, disk creation, free-port allocation.
  - `qmp.rs` — minimal QEMU Monitor Protocol client (shutdown, pause, status,
    snapshots).
  - `vm.rs` — VM config model and on-disk persistence.
  - `session.rs` — records background VMs so a relaunch can reattach to them.
  - `procutil.rs` — Windows PID liveness/terminate helpers for reattached VMs.
  - `lib.rs` — runtime state and the Tauri commands the frontend calls.

The embedded display uses **VNC** (QEMU's built-in VNC server with a websocket
listener, which noVNC connects to directly — no separate proxy). Control runs
over **QMP** (QEMU Monitor Protocol) on a TCP socket. Each VM *also* runs a
**SPICE** server on loopback; the **Open in virt-viewer** button launches the
external `remote-viewer` client against it (richer features the in-window VNC
path can't do — clipboard, dynamic resize, USB redirection).

### Where VMs live

```
%APPDATA%/com.yodawg.app/
├── running.json              # VMs left running/paused in the background
└── machines/<name>/
    ├── vm.json               # VM config (RAM, CPU, disk, ISO, adapters, port forwards, shared folder, ...)
    ├── disk.qcow2            # virtual disk (also holds snapshots)
    └── qemu.log             # QEMU stdout/stderr from the last launch
```

## Moving files in and out of a VM

Two built-in ways to get files across the host/guest boundary, no extra tools
on the host:

**Shared folder (vvfat).** In the create or edit dialog, pick a **Shared
folder**. On the next start, that host folder appears inside the guest as a
small extra disk formatted FAT:

- **Linux guests:** it shows up as a second disk (e.g. `/dev/sdb1`) — mount it
  with `mount /dev/sdb1 /mnt` (the partition is FAT, type `vfat`).
- **Windows guests:** it auto-assigns a new drive letter.
- **DOS guests:** it's an extra drive letter (e.g. `D:`).

Anything you drop in the host folder shows up in the guest. It's **read-only by
default** — solid and safe for copying files *in*. Ticking **Allow the guest to
write back** enables vvfat's read-write mode so the guest can save files back to
the host folder, but that mode is fragile and **can corrupt the folder's
contents** under heavy or concurrent writes, so treat it as experimental and
don't point it at anything irreplaceable. Changes to the shared-folder setting
take effect the next time the VM starts.

> vvfat presents a *snapshot* of the folder taken at boot. If you add files to
> the host folder while the guest is already running, restart the guest (or
> remount the drive inside it) to see them.

**Port forwarding.** For a robust two-way channel — especially for large files —
add a host→guest **port forward** under Networking and run a server in the
guest: e.g. an SSH/SFTP server (then `scp` to `127.0.0.1:<hostPort>` from the
host) or a quick `python3 -m http.server` to pull files over a browser. This
needs the guest online (NAT or Isolated mode — port forwards work in both).

## Troubleshooting

### FreeDOS (or other DOS) won't boot after removing the install CD

The VM boots disk-first with CD-ROM fallback. If a DOS guest only boots while
the install ISO is attached and fails ("no bootable device" / "Invalid partition
table") once you detach it, the installer never wrote boot code to the disk's
**master boot record** — so the BIOS skips the disk and was really booting from
the CD all along, which then chained into the disk.

Fix it from inside the guest, one time:

1. Boot **with the ISO still attached** to reach a DOS prompt.
2. Run:
   ```
   FDISK /MBR     REM write standard MBR boot code to the first hard disk
   SYS C:         REM (re)install the boot sector + kernel on C:
   ```
   In `FDISK`, also confirm partition 1 is set **Active** (option 2).
3. Shut down, **Detach ISO**, and boot — it should now boot standalone to `C:`.

### Mouse drifts or won't reach the screen edges (DOS, Windows 3.1)

Older guests with only a relative pointing device can't track an absolute
cursor over VNC. Click the **1:1** toggle (top-right of the display) so the
framebuffer renders at native scale and pointer deltas map cleanly.

### virt-viewer won't auto-resize the guest (esp. GNOME / Wayland)

Auto-resize only works in the **virt-viewer (SPICE)** window — not the embedded
noVNC display — and needs the guest's `spice-vdagent` installed and the VM set
to the **QXL** or **VirtIO** display adapter. First, in virt-viewer, make sure
**View → Automatically resize** is checked.

If it still won't track — classically it *fits once on connect/login but then
ignores window resizes*, while the login screen still resizes fine — the guest
desktop has **pinned its resolution**. On GNOME (Wayland especially), the
compositor stops following the SPICE size hint once a fixed resolution is saved.
Fix it inside the guest:

```bash
rm ~/.config/monitors.xml     # remove the pinned monitor config
# then log out and back in
```

Avoid setting an explicit resolution in **Settings → Displays** afterward —
GNOME rewrites `monitors.xml` and re-pins it. Leave the display on its default
and it'll keep following the virt-viewer window size.

## Roadmap

- More networking beyond the current NAT / Isolated / port-forwarding: bridged
  or host-only (guest on the physical LAN — needs a TAP driver + admin on
  Windows) and VM-to-VM internal networks
- macOS (HVF) and Linux (KVM) support
- Disk resize, VM cloning, OVA/OVF import/export

## License

[MIT](LICENSE) © Jeff Aigner
</content>
</invoke>
