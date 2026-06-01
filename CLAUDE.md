# CLAUDE.md

Guidance for working in this repository.

## Working principles

- **Ask, don't assume.** If something's unclear, ask before writing a line. No
  silent guesses about intent, architecture, or requirements.
- **Simplest solution first.** Implement the minimum thing that works. No
  abstractions you didn't request.
- **Don't touch unrelated code.** If a file isn't part of the current task,
  leave it.
- **Flag uncertainty explicitly.** If you're not confident, say so before
  proceeding — confidence without certainty causes more damage than admitting a
  gap.

## What this is

**yodawg** is a friendly GUI wrapper around QEMU — see [README.md](./README.md).
Stack: **Tauri v2 + React/TypeScript** frontend, **Rust** backend that spawns and
controls `qemu-system-x86_64`. Primary target is **Windows native** (WHPX
acceleration); the display is **embedded noVNC** over QEMU's VNC websocket.

## ⚠️ Build environment: WSL targeting Windows

Development often happens from **WSL**, but the app is **Windows-native** (it uses
WebView2 and launches the Windows QEMU `.exe`s). Build/run the Windows toolchain
from WSL via interop, and respect these hard-won rules:

- **Keep the shell CWD on the Windows filesystem** (`/mnt/c/...`). Windows
  `cargo.exe`/`node.exe` fail against WSL paths like `/tmp` ("could not create
  session directory lock file", "Cannot find module C:\tmp\...").
- **`npm` is `npm.cmd`** — bash can't exec it. Use `cmd.exe /c "npm ..."`.
  `node.exe` and `cargo.exe` run directly.
- **Don't pass WSL paths as args to `.exe`s.** Rely on CWD + relative paths, or
  pass real `C:\...` paths.
- The app build must be a **Windows** build (`x86_64-pc-windows-msvc`). Don't
  `cargo build` a Linux binary in WSL and expect QEMU/WebView2 to work.

Installed on the Windows side: QEMU 11 at `C:\Program Files\qemu` (not on PATH),
Rust (MSVC), Node, WebView2. Override QEMU location with `YODAWG_QEMU_DIR`.

## Commands

```bash
cmd.exe /c "npm install"            # install frontend deps
cmd.exe /c "npm run tauri dev"      # run with hot reload (opens a Windows window)
cmd.exe /c "npm run build"          # typecheck + bundle frontend (fast sanity check)
cd src-tauri && cargo.exe check     # check the Rust backend (run with CWD in src-tauri)
```

- Frontend changes hot-reload via Vite. **Backend (Rust) changes** trigger a
  `cargo` recompile and **relaunch the app** — `tauri dev` watches `src-tauri/`.
- `npm run tauri dev` opens a GUI window on the user's desktop; an agent can't
  see it. Verify backend behavior structurally (compile, process checks via
  `tasklist.exe`/`taskkill.exe`) and ask the user to confirm the visual side.

## Architecture

Frontend (`src/`):
- `App.tsx` — sidebar VM list + toolbar lifecycle controls + display panel.
- `CreateVmDialog.tsx`, `EditVmDialog.tsx` — modal forms.
- `VncViewer.tsx` — noVNC client; connects to `ws://127.0.0.1:<port>`.
- `api.ts` — typed wrappers over `invoke(...)` and the dialog plugin.

Backend (`src-tauri/src/`):
- `lib.rs` — `AppState` (running-VM map, close flag), all `#[tauri::command]`s,
  the window-close **pause-and-exit** handler, and `reconcile_running` (reattach
  on launch).
- `qemu.rs` — binary discovery, accel/CPU selection, **argument building**,
  disk creation, free-port allocation. **All QEMU flag knowledge lives here.**
- `qmp.rs` — minimal synchronous QMP (QEMU Monitor Protocol) client over TCP.
- `vm.rs` — `VmConfig` model + persistence under `app_data/machines/<name>/`.
- `session.rs` — persists the set of background VMs (`app_data/running.json`)
  so a relaunch can reattach.
- `procutil.rs` — Windows PID liveness/terminate helpers (for reattached VMs we
  don't hold a `Child` for); shells out to `kill` elsewhere.

Commands exposed to the frontend: `detect_qemu`, `list_vms`, `create_vm`,
`update_vm`, `detach_iso`, `delete_vm`, `start_vm`, `running_info`, `stop_vm`,
`pause_vm`, `resume_vm`, `force_kill_vm`, `live_snapshots_supported`,
`list_snapshots`, `create_snapshot`, `restore_snapshot`, `delete_snapshot`.

**VM lifecycle across app restarts:** spawned QEMU processes deliberately
**outlive the app** — they are *not* tied to a kill-on-close Job Object, and
their stdio goes to a per-VM `qemu.log` (not a pipe yodawg owns). Closing the
window **pauses** running guests (QMP `stop`) and records them to
`running.json`; on the next launch `reconcile_running` probes each (PID alive +
QMP `query-name` matches the `-name` we set) and reattaches the survivors,
which show as running/paused and can be resumed. The old "no orphans / die with
the app" guarantee is intentionally gone; cleanup happens via reconcile on the
next launch instead.

## Conventions & key facts

- **Tauri v2 arg casing:** JS passes **camelCase** keys; Tauri maps them to the
  Rust command's `snake_case` params. e.g. `invoke("create_vm", { memoryMb })`
  → `fn create_vm(memory_mb: u32)`. Single-word args (`name`) need no mapping.
- **Display:** QEMU launches with `-display none -vnc 127.0.0.1:D,websocket=P`;
  noVNC connects to port `P`. No proxy needed — QEMU's VNC server speaks the
  websocket directly.
- **Control:** QMP over TCP (`-qmp tcp:127.0.0.1:Q,server,nowait`). **Not** a
  unix socket — that's Linux-only and won't work on Windows.
- **Windows quirks handled in `qemu.rs`:** `-accel whpx` + `-cpu qemu64`
  (`-cpu max`/`host` crash WHPX); `-drive file=...,format=qcow2` (the `file=`
  prefix is mandatory); `-usb -device usb-tablet` (absolute pointer, else the
  cursor drifts over VNC); `-boot order=cd` (disk-first, CD fallback).
- **Networking (`qemu.rs`):** per-VM `net_mode` — `nat` (user-mode NAT,
  default), `isolated` (`-netdev user,...,restrict=on` — guest gets a DHCP lease
  but can't reach host/internet; `hostfwd` rules still work), or `none`
  (`-nic none`, no card). Port forwards become `hostfwd=proto::host-:guest`
  rules on the netdev. NIC model maps to a `-device` (`e1000`/`virtio-net-pci`/
  `rtl8139`/`ne2k_isa`); `ne2k` is for DOS guests. Each VM has a **stable MAC**
  (`52:54:00:xx:xx:xx`, `qemu::derive_mac` from the name) persisted in `vm.json`
  and passed as `mac=` so DHCP leases / MAC-bound licenses survive reboots; old
  configs are backfilled on save and at arg-build time.
- **VMs outlive the app:** spawned QEMU is *not* killed on close. Window-close
  **pauses** running guests (QMP `stop`) and records them to `running.json`; the
  next launch reattaches them (see the lifecycle note under Architecture). The
  cost is that a VM can keep running if yodawg is never reopened — reconcile on
  launch is the cleanup, not a kill-on-close Job Object.
- **Adding a command:** write the `#[tauri::command]` in `lib.rs`, add it to the
  `generate_handler![...]` list, then add a wrapper in `src/api.ts`. New plugin
  permissions go in `src-tauri/capabilities/default.json`.

## Footguns

- **Never run directory-wiping commands near the user's files.** A
  `create-tauri-app --force` once permanently deleted a user's ISO and disk
  image (it wipes the target dir, bypassing the Recycle Bin). Move user data
  fully out of the tree first, scaffold into an empty subdir, and `ls` to verify
  before/after. Treat ISOs and disk images as irreplaceable.
- Large test artifacts (ISOs, `disk.qcow2`) belong in `_testdata/` (gitignored).
- `src-tauri/target/`, `node_modules/`, `dist/` are gitignored — don't commit them.

## Git

Remote: `git@github.com:jaigner-hub/yodawg.git`. The repo is configured with
`core.autocrlf=false` and `core.fileMode=false` (the working tree is on an NTFS
mount under WSL). Commit/push only when asked.
