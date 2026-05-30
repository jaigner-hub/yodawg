# yodawg

A friendly, cross-platform GUI for [QEMU](https://www.qemu.org/) — a VirtualBox-like
experience with QEMU/KVM performance underneath.

QEMU is fast and powerful but its command line is intimidating and there's no
great desktop UI for it. yodawg wraps QEMU so normal people can create, run, and
manage virtual machines without touching a flag, while still getting native
hardware acceleration (WHPX on Windows, and KVM/HVF later).

> Status: **v0.1 (MVP)**. Primary target today is **Windows native** (WHPX).
> Working codename, subject to change.

## Features

- **Create-VM wizard** — pick an ISO, set RAM / CPU / disk size, and it creates
  the `qcow2` disk and boots.
- **VM list** with live running/stopped status.
- **Embedded display** — the VM renders right inside the app window (QEMU VNC
  websocket + [noVNC](https://novnc.com/)).
- **Lifecycle controls** — start, graceful ACPI shutdown, force kill, delete.
- **Edit settings** — change RAM, CPUs, and the attached ISO of a stopped VM;
  eject the ISO.
- **Just works defaults** — acceleration and a safe CPU model are auto-selected
  per platform; absolute-pointer mouse (USB tablet) so the cursor tracks 1:1;
  disk-first boot order so installed systems boot themselves.
- **Safe shutdown** — closing the window cleanly powers down running guests; VMs
  can't be orphaned even if the app crashes.

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

## How it works

- **Frontend** (`src/`) — React + TypeScript in a Tauri webview. Renders the VM
  list, the create/edit dialogs, and the embedded noVNC viewer.
- **Backend** (`src-tauri/src/`) — Rust. Spawns and tracks `qemu-system-x86_64`
  processes, controls them over QMP, and persists VM configs.
  - `qemu.rs` — binary discovery, acceleration/CPU selection, QEMU argument
    building, disk creation, free-port allocation.
  - `qmp.rs` — minimal QEMU Monitor Protocol client (shutdown, status).
  - `vm.rs` — VM config model and on-disk persistence.
  - `proc_job.rs` — Windows Job Object so QEMU dies with the app.
  - `lib.rs` — runtime state and the Tauri commands the frontend calls.

The display uses **VNC** (QEMU's built-in VNC server with a websocket listener,
which noVNC connects to directly — no separate proxy). SPICE is a future option.

### Where VMs live

```
%APPDATA%/com.yodawg.app/machines/<name>/
├── vm.json        # VM config (RAM, CPU, disk path, ISO, ...)
└── disk.qcow2     # virtual disk
```

## Roadmap

- Snapshots (save/restore VM state)
- Networking options (port forwarding, NAT / bridged / host-only)
- SPICE protocol (clipboard sharing, auto display-resize, USB redirection)
- macOS (HVF) and Linux (KVM) support
- Disk resize, VM cloning, OVA/OVF import/export

## License

TBD.
