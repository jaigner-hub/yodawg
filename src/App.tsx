import { useEffect, useState, useCallback } from "react";
import { api, VmStatus, RunningInfo } from "./api";
import { CreateVmDialog } from "./CreateVmDialog";
import { EditVmDialog } from "./EditVmDialog";
import { SnapshotsDialog } from "./SnapshotsDialog";
import "./App.css";

// Human-readable labels for the config enums shown in the detail panel.
const ACCEL_LABEL: Record<string, string> = {
  tcg: "Software (TCG)",
  auto: "Hardware",
};
const DISPLAY_LABEL: Record<string, string> = {
  std: "Standard",
  virtio: "VirtIO",
  qxl: "QXL",
};
const NET_LABEL: Record<string, string> = {
  nat: "NAT (internet)",
  isolated: "Isolated",
  none: "No network",
};
const NIC_LABEL: Record<string, string> = {
  e1000: "Intel e1000",
  virtio: "VirtIO",
  rtl8139: "RTL8139",
  ne2k: "NE2000 (ISA)",
};

/** Last path segment of a Windows/Unix path (for showing an ISO/folder name). */
function baseName(p?: string | null): string | null {
  if (!p) return null;
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export default function App() {
  const [vms, setVms] = useState<VmStatus[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [viewer, setViewer] = useState<RunningInfo | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [showEdit, setShowEdit] = useState(false);
  const [showSnapshots, setShowSnapshots] = useState(false);
  const [qemu, setQemu] = useState<{ ok: boolean; text: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await api.listVms();
      setVms(list);
      setSelected((cur) => cur ?? list[0]?.name ?? null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Initial load + QEMU health check, then poll status periodically.
  useEffect(() => {
    api
      .detectQemu()
      .then((text) => setQemu({ ok: true, text }))
      .catch((e) => setQemu({ ok: false, text: String(e) }));
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh]);

  // When the selected VM changes (or status refreshes), attach/detach display.
  useEffect(() => {
    let cancelled = false;
    if (!selected) {
      setViewer(null);
      return;
    }
    api.runningInfo(selected).then((info) => {
      if (!cancelled) setViewer(info);
    });
    return () => {
      cancelled = true;
    };
  }, [selected, vms]);

  const current = vms.find((v) => v.name === selected) ?? null;

  async function withErrors(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function start(name: string) {
    await withErrors(async () => {
      const info = await api.startVm(name);
      setViewer(info);
      // The display lives in virt-viewer (no embedded viewer), so open it
      // automatically once the VM is up.
      await api.openInViewer(name);
    });
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          yo<span>dawg</span>
        </div>

        <button className="new-vm" onClick={() => setShowCreate(true)}>
          + New VM
        </button>

        <ul className="vm-list">
          {vms.map((vm) => (
            <li
              key={vm.name}
              className={vm.name === selected ? "selected" : ""}
              onClick={() => setSelected(vm.name)}
            >
              <span className={`dot ${vm.running ? "on" : "off"}`} />
              <span className="vm-name">{vm.name}</span>
            </li>
          ))}
          {vms.length === 0 && <li className="empty">No VMs yet — create one.</li>}
        </ul>

        {qemu && (
          <div className={`qemu-status ${qemu.ok ? "ok" : "bad"}`}>
            {qemu.ok ? qemu.text : `QEMU not found: ${qemu.text}`}
          </div>
        )}
      </aside>

      <main className="main">
        {current ? (
          <>
            <header className="toolbar">
              <div className="title">
                <h1>{current.name}</h1>
                <span
                  className={`status ${
                    !current.running ? "stopped" : viewer?.paused ? "paused" : "running"
                  }`}
                >
                  {!current.running ? "stopped" : viewer?.paused ? "paused" : "running"}
                </span>
              </div>
              <div className="actions">
                {current.running ? (
                  <>
                    {viewer?.paused ? (
                      <button onClick={() => withErrors(() => api.resumeVm(current.name))}>
                        Resume
                      </button>
                    ) : (
                      <button onClick={() => withErrors(() => api.pauseVm(current.name))}>
                        Pause
                      </button>
                    )}
                    <button onClick={() => withErrors(() => api.stopVm(current.name))}>
                      Shut down
                    </button>
                    <button
                      className="danger"
                      onClick={() => withErrors(() => api.forceKillVm(current.name))}
                    >
                      Force kill
                    </button>
                  </>
                ) : (
                  <button className="primary" onClick={() => start(current.name)}>
                    Start
                  </button>
                )}
              </div>
            </header>

            <section className="detail">
              <div className="card display-card">
                <div className="card-head">Display</div>
                {current.running ? (
                  <div className="display-launch">
                    <button
                      className="primary"
                      onClick={() => withErrors(() => api.openInViewer(current.name))}
                    >
                      ▶ Open display
                    </button>
                    <p className="sub">
                      Opens in a separate virt-viewer window. Closing it just
                      disconnects — the VM keeps running.
                    </p>
                  </div>
                ) : (
                  <p className="sub">
                    Start the VM to open its display in virt-viewer.
                  </p>
                )}
              </div>

              <div className="specs-grid">
                <div className="spec-col">
                  <h3>System</h3>
                  <dl>
                    <div>
                      <dt>CPU</dt>
                      <dd>{current.cpus} vCPU</dd>
                    </div>
                    <div>
                      <dt>Memory</dt>
                      <dd>{current.memory_mb} MB</dd>
                    </div>
                    <div>
                      <dt>Acceleration</dt>
                      <dd>{ACCEL_LABEL[current.acceleration] ?? current.acceleration}</dd>
                    </div>
                    <div>
                      <dt>Display</dt>
                      <dd>{DISPLAY_LABEL[current.display_adapter] ?? current.display_adapter}</dd>
                    </div>
                  </dl>
                </div>

                <div className="spec-col">
                  <h3>Network</h3>
                  <dl>
                    <div>
                      <dt>Mode</dt>
                      <dd>{NET_LABEL[current.net_mode] ?? current.net_mode}</dd>
                    </div>
                    {current.net_mode !== "none" && (
                      <>
                        <div>
                          <dt>Adapter</dt>
                          <dd>{NIC_LABEL[current.nic_model] ?? current.nic_model}</dd>
                        </div>
                        {current.mac_address && (
                          <div>
                            <dt>MAC</dt>
                            <dd className="mono">{current.mac_address}</dd>
                          </div>
                        )}
                        <div>
                          <dt>Forwards</dt>
                          <dd>
                            {current.port_forwards.length === 0
                              ? "—"
                              : current.port_forwards
                                  .map(
                                    (f) =>
                                      `${f.host_port}→${f.guest_port}${
                                        f.protocol === "udp" ? "/udp" : ""
                                      }`,
                                  )
                                  .join(", ")}
                          </dd>
                        </div>
                      </>
                    )}
                  </dl>
                </div>

                <div className="spec-col">
                  <h3>Storage</h3>
                  <dl>
                    <div>
                      <dt>Disk</dt>
                      <dd>{current.disk_size_gb} GB · qcow2</dd>
                    </div>
                    <div>
                      <dt>ISO</dt>
                      <dd>{baseName(current.iso_path) ?? "none"}</dd>
                    </div>
                    <div>
                      <dt>Shared</dt>
                      <dd>
                        {baseName(current.shared_folder)
                          ? `${baseName(current.shared_folder)}${
                              current.shared_folder_writable ? " (rw)" : " (ro)"
                            }`
                          : "—"}
                      </dd>
                    </div>
                  </dl>
                </div>
              </div>

              <div className="detail-actions">
                <button onClick={() => setShowSnapshots(true)}>Snapshots</button>
                {!current.running && (
                  <button onClick={() => setShowEdit(true)}>Edit</button>
                )}
                {!current.running && current.iso_path && (
                  <button
                    title="Boots from the disk only. DOS guests (e.g. FreeDOS) may not boot afterward if the installer didn't write the MBR — boot once with the ISO attached and run FDISK /MBR then SYS C: first."
                    onClick={() => withErrors(() => api.detachIso(current.name))}
                  >
                    Eject ISO
                  </button>
                )}
                {!current.running && (
                  <button
                    className="danger"
                    onClick={() => withErrors(() => api.deleteVm(current.name))}
                  >
                    Delete
                  </button>
                )}
              </div>
            </section>
          </>
        ) : (
          <div className="placeholder big">
            Select a VM, or create one to get started.
          </div>
        )}

        {error && (
          <div className="toast" onClick={() => setError(null)}>
            {error}
          </div>
        )}
      </main>

      {showCreate && (
        <CreateVmDialog
          onClose={() => setShowCreate(false)}
          onCreated={(name) => {
            setShowCreate(false);
            setSelected(name);
            refresh();
          }}
        />
      )}

      {showEdit && current && (
        <EditVmDialog
          vm={current}
          onClose={() => setShowEdit(false)}
          onSaved={() => {
            setShowEdit(false);
            refresh();
          }}
        />
      )}

      {showSnapshots && current && (
        <SnapshotsDialog vm={current} onClose={() => setShowSnapshots(false)} />
      )}
    </div>
  );
}
