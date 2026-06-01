// Typed wrappers around the Tauri backend commands (see src-tauri/src/lib.rs).
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

// Keys are snake_case to match the Rust serde struct: these objects are passed
// through to serde as-is (nested values aren't camelCase-mapped by Tauri).
export interface PortForward {
  host_port: number;
  guest_port: number;
  protocol: string; // "tcp" | "udp"
}

export interface VmStatus {
  name: string;
  memory_mb: number;
  cpus: number;
  disk_path: string;
  disk_size_gb: number;
  iso_path?: string | null;
  display_adapter: string;
  nic_model: string;
  port_forwards: PortForward[];
  net_mode: string; // "nat" | "isolated" | "none"
  mac_address?: string | null;
  running: boolean;
  websocket_port?: number | null;
}

export interface Snapshot {
  tag: string;
  // Saved machine-state size in bytes; 0 = disk-only (taken while stopped).
  vm_state_size: number;
  // Unix timestamp (seconds) the snapshot was taken.
  date_sec: number;
}

export interface RunningInfo {
  name: string;
  websocket_port: number;
  vnc_display: number;
  qmp_port: number;
  paused: boolean;
}

export interface CreateVmParams {
  name: string;
  memory_mb: number;
  cpus: number;
  disk_size_gb: number;
  iso_path?: string | null;
  display_adapter: string;
  nic_model: string;
  port_forwards: PortForward[];
  net_mode: string; // "nat" | "isolated" | "none"
}

export const api = {
  detectQemu: () => invoke<string>("detect_qemu"),
  listVms: () => invoke<VmStatus[]>("list_vms"),
  // Tauri v2 maps camelCase JS keys to the Rust command's snake_case params.
  // (Nested objects like port_forwards keep snake_case — serde reads them raw.)
  createVm: (p: CreateVmParams) =>
    invoke<void>("create_vm", {
      name: p.name,
      memoryMb: p.memory_mb,
      cpus: p.cpus,
      diskSizeGb: p.disk_size_gb,
      isoPath: p.iso_path ?? null,
      displayAdapter: p.display_adapter,
      nicModel: p.nic_model,
      portForwards: p.port_forwards,
      netMode: p.net_mode,
    }),
  updateVm: (p: {
    name: string;
    memory_mb: number;
    cpus: number;
    iso_path?: string | null;
    display_adapter: string;
    nic_model: string;
    port_forwards: PortForward[];
    net_mode: string;
  }) =>
    invoke<void>("update_vm", {
      name: p.name,
      memoryMb: p.memory_mb,
      cpus: p.cpus,
      isoPath: p.iso_path ?? null,
      displayAdapter: p.display_adapter,
      nicModel: p.nic_model,
      portForwards: p.port_forwards,
      netMode: p.net_mode,
    }),
  detachIso: (name: string) => invoke<void>("detach_iso", { name }),
  deleteVm: (name: string) => invoke<void>("delete_vm", { name }),
  startVm: (name: string) => invoke<RunningInfo>("start_vm", { name }),
  stopVm: (name: string) => invoke<void>("stop_vm", { name }),
  pauseVm: (name: string) => invoke<void>("pause_vm", { name }),
  resumeVm: (name: string) => invoke<void>("resume_vm", { name }),
  forceKillVm: (name: string) => invoke<void>("force_kill_vm", { name }),
  runningInfo: (name: string) =>
    invoke<RunningInfo | null>("running_info", { name }),

  // Snapshots. Single-word args (name/tag) need no camelCase mapping.
  liveSnapshotsSupported: () =>
    invoke<boolean>("live_snapshots_supported"),
  listSnapshots: (name: string) =>
    invoke<Snapshot[]>("list_snapshots", { name }),
  createSnapshot: (name: string, tag: string) =>
    invoke<void>("create_snapshot", { name, tag }),
  restoreSnapshot: (name: string, tag: string) =>
    invoke<void>("restore_snapshot", { name, tag }),
  deleteSnapshot: (name: string, tag: string) =>
    invoke<void>("delete_snapshot", { name, tag }),

  // Native file picker for choosing an install ISO.
  pickIso: () =>
    open({
      multiple: false,
      directory: false,
      filters: [{ name: "Disk image", extensions: ["iso", "img"] }],
    }) as Promise<string | null>,
};
