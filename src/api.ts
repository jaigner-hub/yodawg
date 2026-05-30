// Typed wrappers around the Tauri backend commands (see src-tauri/src/lib.rs).
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export interface VmStatus {
  name: string;
  memory_mb: number;
  cpus: number;
  disk_path: string;
  disk_size_gb: number;
  iso_path?: string | null;
  running: boolean;
  websocket_port?: number | null;
}

export interface RunningInfo {
  name: string;
  websocket_port: number;
  vnc_display: number;
  qmp_port: number;
}

export interface CreateVmParams {
  name: string;
  memory_mb: number;
  cpus: number;
  disk_size_gb: number;
  iso_path?: string | null;
}

export const api = {
  detectQemu: () => invoke<string>("detect_qemu"),
  listVms: () => invoke<VmStatus[]>("list_vms"),
  // Tauri v2 maps camelCase JS keys to the Rust command's snake_case params.
  createVm: (p: CreateVmParams) =>
    invoke<void>("create_vm", {
      name: p.name,
      memoryMb: p.memory_mb,
      cpus: p.cpus,
      diskSizeGb: p.disk_size_gb,
      isoPath: p.iso_path ?? null,
    }),
  updateVm: (p: { name: string; memory_mb: number; cpus: number; iso_path?: string | null }) =>
    invoke<void>("update_vm", {
      name: p.name,
      memoryMb: p.memory_mb,
      cpus: p.cpus,
      isoPath: p.iso_path ?? null,
    }),
  detachIso: (name: string) => invoke<void>("detach_iso", { name }),
  deleteVm: (name: string) => invoke<void>("delete_vm", { name }),
  startVm: (name: string) => invoke<RunningInfo>("start_vm", { name }),
  stopVm: (name: string) => invoke<void>("stop_vm", { name }),
  forceKillVm: (name: string) => invoke<void>("force_kill_vm", { name }),
  runningInfo: (name: string) =>
    invoke<RunningInfo | null>("running_info", { name }),

  // Native file picker for choosing an install ISO.
  pickIso: () =>
    open({
      multiple: false,
      directory: false,
      filters: [{ name: "Disk image", extensions: ["iso", "img"] }],
    }) as Promise<string | null>,
};
