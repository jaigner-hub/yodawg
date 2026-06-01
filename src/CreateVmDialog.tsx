import { useState } from "react";
import { api, PortForward } from "./api";
import {
  DisplaySelect,
  AccelSelect,
  NicSelect,
  NetModeSelect,
  PortForwardsEditor,
} from "./SettingsFields";

/** Modal form that collects the fields needed to create + boot a new VM. */
export function CreateVmDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (name: string) => void;
}) {
  const [name, setName] = useState("");
  const [memoryMb, setMemoryMb] = useState(4096);
  const [cpus, setCpus] = useState(2);
  const [diskGb, setDiskGb] = useState(20);
  const [isoPath, setIsoPath] = useState<string | null>(null);
  const [displayAdapter, setDisplayAdapter] = useState("std");
  const [acceleration, setAcceleration] = useState("auto");
  const [nicModel, setNicModel] = useState("e1000");
  const [netMode, setNetMode] = useState("nat");
  const [portForwards, setPortForwards] = useState<PortForward[]>([]);
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
      await api.createVm({
        name: name.trim(),
        memory_mb: memoryMb,
        cpus,
        disk_size_gb: diskGb,
        iso_path: isoPath,
        display_adapter: displayAdapter,
        nic_model: nicModel,
        port_forwards: portForwards,
        net_mode: netMode,
        acceleration,
      });
      onCreated(name.trim());
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Create a VM</h2>

        <label>
          Name
          <input
            autoFocus
            value={name}
            placeholder="ubuntu-desktop"
            onChange={(e) => setName(e.target.value)}
          />
        </label>

        <label>
          Install ISO
          <div className="iso-row">
            <input
              readOnly
              value={isoPath ?? ""}
              placeholder="(optional — pick an .iso to boot from)"
            />
            <button type="button" onClick={chooseIso}>
              Browse…
            </button>
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
          <label>
            Disk (GB)
            <input
              type="number"
              min={1}
              value={diskGb}
              onChange={(e) => setDiskGb(Number(e.target.value))}
            />
          </label>
        </div>

        <DisplaySelect value={displayAdapter} onChange={setDisplayAdapter} />
        <AccelSelect value={acceleration} onChange={setAcceleration} />
        <NetModeSelect value={netMode} onChange={setNetMode} />
        {netMode !== "none" && (
          <>
            <NicSelect value={nicModel} onChange={setNicModel} />
            <PortForwardsEditor
              value={portForwards}
              onChange={setPortForwards}
            />
          </>
        )}

        {error && <p className="error">{error}</p>}

        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            className="primary"
            onClick={submit}
            disabled={busy || name.trim() === ""}
          >
            {busy ? "Creating…" : "Create VM"}
          </button>
        </div>
      </div>
    </div>
  );
}
