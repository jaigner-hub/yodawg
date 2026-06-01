import { useCallback, useEffect, useState } from "react";
import { api, Snapshot, VmStatus } from "./api";

/**
 * Snapshot manager for a VM. Lists the qcow2 internal snapshots and allows
 * creating / restoring / deleting them. When the VM is running, snapshots
 * capture live RAM (via QMP `savevm`); when stopped, they are disk-only.
 */
export function SnapshotsDialog({
  vm,
  onClose,
}: {
  vm: VmStatus;
  onClose: () => void;
}) {
  const [snaps, setSnaps] = useState<Snapshot[]>([]);
  const [tag, setTag] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [liveSupported, setLiveSupported] = useState(true);

  // Snapshotting a *running* VM needs migratable state, which WHPX/HVF lack —
  // when that's the case, creating/restoring is blocked until the VM is stopped.
  const liveBlocked = vm.running && !liveSupported;

  const refresh = useCallback(async () => {
    try {
      setSnaps(await api.listSnapshots(vm.name));
    } catch (e) {
      setError(String(e));
    }
  }, [vm.name]);

  useEffect(() => {
    api.liveSnapshotsSupported(vm.name).then(setLiveSupported).catch(() => {});
    refresh();
  }, [refresh, vm.name]);

  // Run a snapshot op, then refresh the list. `busy` guards against overlap.
  async function run(fn: () => Promise<unknown>) {
    setError(null);
    setBusy(true);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function create() {
    const t = tag.trim();
    if (!t) return;
    await run(async () => {
      await api.createSnapshot(vm.name, t);
      setTag("");
    });
  }

  async function restore(t: string) {
    if (
      !confirm(
        `Restore "${t}"? This discards ${vm.name}'s current state and replaces it with the snapshot.`
      )
    )
      return;
    await run(() => api.restoreSnapshot(vm.name, t));
  }

  async function remove(t: string) {
    if (!confirm(`Delete snapshot "${t}"? This can't be undone.`)) return;
    await run(() => api.deleteSnapshot(vm.name, t));
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Snapshots — {vm.name}</h2>

        {liveBlocked ? (
          <p className="error">
            This VM is running with an accelerator (WHPX / HVF) that can't take
            live snapshots. Shut it down to snapshot its disk.
          </p>
        ) : (
          <p className="hint">
            {vm.running
              ? "VM is running — new snapshots capture live RAM and can be restored instantly."
              : "VM is stopped — snapshots capture the disk state."}
          </p>
        )}

        <div className="snap-create">
          <input
            placeholder="Snapshot name"
            value={tag}
            disabled={busy || liveBlocked}
            onChange={(e) => setTag(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && create()}
          />
          <button
            className="primary"
            onClick={create}
            disabled={busy || liveBlocked || !tag.trim()}
          >
            Take snapshot
          </button>
        </div>

        <ul className="snap-list">
          {snaps.map((s) => (
            <li key={s.tag}>
              <div className="snap-info">
                <span className="snap-tag">{s.tag}</span>
                <span className="snap-meta">
                  {s.date_sec ? new Date(s.date_sec * 1000).toLocaleString() : "—"}
                  {s.vm_state_size > 0 ? " · with RAM" : " · disk only"}
                </span>
              </div>
              <div className="snap-actions">
                <button onClick={() => restore(s.tag)} disabled={busy || liveBlocked}>
                  Restore
                </button>
                <button
                  className="danger"
                  onClick={() => remove(s.tag)}
                  disabled={busy}
                >
                  Delete
                </button>
              </div>
            </li>
          ))}
          {snaps.length === 0 && (
            <li className="empty">No snapshots yet.</li>
          )}
        </ul>

        {error && <p className="error">{error}</p>}

        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
