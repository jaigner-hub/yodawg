import { useState } from "react";
import { api, VmStatus, PortForward } from "./api";
import {
  DisplaySelect,
  AccelSelect,
  NicSelect,
  NetModeSelect,
  PortForwardsEditor,
} from "./SettingsFields";

/** Modal form for editing a stopped VM's settings (memory, CPUs, ISO). */
export function EditVmDialog({
  vm,
  onClose,
  onSaved,
}: {
  vm: VmStatus;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [memoryMb, setMemoryMb] = useState(vm.memory_mb);
  const [cpus, setCpus] = useState(vm.cpus);
  const [isoPath, setIsoPath] = useState<string | null>(vm.iso_path ?? null);
  const [displayAdapter, setDisplayAdapter] = useState(vm.display_adapter ?? "std");
  const [acceleration, setAcceleration] = useState(vm.acceleration ?? "auto");
  const [nicModel, setNicModel] = useState(vm.nic_model ?? "e1000");
  const [netMode, setNetMode] = useState(vm.net_mode ?? "nat");
  const [portForwards, setPortForwards] = useState<PortForward[]>(
    vm.port_forwards ?? []
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function chooseIso() {
    const picked = await api.pickIso();
    if (picked) setIsoPath(picked);
  }

  async function submit() {
    setError(null);
    setBusy(true);
    try {
      await api.updateVm({
        name: vm.name,
        memory_mb: memoryMb,
        cpus,
        iso_path: isoPath,
        display_adapter: displayAdapter,
        nic_model: nicModel,
        port_forwards: portForwards,
        net_mode: netMode,
        acceleration,
      });
      onSaved();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Settings — {vm.name}</h2>

        <label>
          Disk
          <input
            readOnly
            value={`${vm.disk_size_gb} GB  (disk size can't be changed here)`}
          />
        </label>

        <label>
          Install ISO
          <div className="iso-row">
            <input
              readOnly
              value={isoPath ?? ""}
              placeholder="(no ISO attached)"
            />
            <button type="button" onClick={chooseIso}>
              Browse…
            </button>
            {isoPath && (
              <button type="button" onClick={() => setIsoPath(null)}>
                Clear
              </button>
            )}
          </div>
        </label>

        <div className="field-row">
          <label>
            Memory (MB)
            <input
              type="number"
              min={256}
              step={256}
              value={memoryMb}
              onChange={(e) => setMemoryMb(Number(e.target.value))}
            />
          </label>
          <label>
            CPUs
            <input
              type="number"
              min={1}
              max={64}
              value={cpus}
              onChange={(e) => setCpus(Number(e.target.value))}
            />
          </label>
        </div>

        <DisplaySelect value={displayAdapter} onChange={setDisplayAdapter} />
        <AccelSelect value={acceleration} onChange={setAcceleration} />
        <NetModeSelect value={netMode} onChange={setNetMode} />
        {netMode !== "none" && (
          <>
            <NicSelect value={nicModel} onChange={setNicModel} />
            {vm.mac_address && (
              <label>
                MAC address
                <input readOnly value={vm.mac_address} />
              </label>
            )}
            <PortForwardsEditor
              value={portForwards}
              onChange={setPortForwards}
            />
          </>
        )}

        <p className="hint">Changes take effect the next time the VM starts.</p>
        {error && <p className="error">{error}</p>}

        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="primary" onClick={submit} disabled={busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
