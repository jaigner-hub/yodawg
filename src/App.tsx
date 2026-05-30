import { useEffect, useState, useCallback } from "react";
import { api, VmStatus, RunningInfo } from "./api";
import { VncViewer } from "./VncViewer";
import { CreateVmDialog } from "./CreateVmDialog";
import { EditVmDialog } from "./EditVmDialog";
import { SnapshotsDialog } from "./SnapshotsDialog";
import "./App.css";

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
                <span className="specs">
                  {current.cpus} vCPU · {current.memory_mb} MB · {current.disk_size_gb} GB
                  {current.running
                    ? viewer?.paused
                      ? " · paused"
                      : " · running"
                    : " · stopped"}
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
                    <button onClick={() => setShowSnapshots(true)}>Snapshots</button>
                  </>
                ) : (
                  <>
                    <button className="primary" onClick={() => start(current.name)}>
                      Start
                    </button>
                    <button onClick={() => setShowEdit(true)}>Edit</button>
                    <button onClick={() => setShowSnapshots(true)}>Snapshots</button>
                    {current.iso_path && (
                      <button onClick={() => withErrors(() => api.detachIso(current.name))}>
                        Eject ISO
                      </button>
                    )}
                    <button
                      className="danger"
                      onClick={() => withErrors(() => api.deleteVm(current.name))}
                    >
                      Delete
                    </button>
                  </>
                )}
              </div>
            </header>

            <section className="display">
              {viewer ? (
                <VncViewer key={viewer.websocket_port} port={viewer.websocket_port} />
              ) : (
                <div className="placeholder">
                  {current.running
                    ? "Attaching to display…"
                    : "VM is stopped. Press Start to boot it."}
                </div>
              )}
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
