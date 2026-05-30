import { PortForward } from "./api";

/** Display-adapter (VGA model) picker, shared by the create/edit dialogs. */
export function DisplaySelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <label>
      Display adapter
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="std">Standard — most compatible</option>
        <option value="virtio">VirtIO — faster (Linux guests)</option>
      </select>
    </label>
  );
}

/** Editor for host→guest port forwards over the user-mode NAT. */
export function PortForwardsEditor({
  value,
  onChange,
}: {
  value: PortForward[];
  onChange: (v: PortForward[]) => void;
}) {
  function update(i: number, patch: Partial<PortForward>) {
    const next = value.slice();
    next[i] = { ...next[i], ...patch };
    onChange(next);
  }
  function add() {
    onChange([...value, { host_port: 2222, guest_port: 22, protocol: "tcp" }]);
  }
  function remove(i: number) {
    onChange(value.filter((_, j) => j !== i));
  }

  return (
    <label>
      Port forwarding <span className="sub">host → guest</span>
      <div className="pf-list">
        {value.map((pf, i) => (
          <div className="pf-row" key={i}>
            <input
              type="number"
              min={1}
              max={65535}
              value={pf.host_port}
              onChange={(e) => update(i, { host_port: Number(e.target.value) })}
            />
            <span className="pf-arrow">→</span>
            <input
              type="number"
              min={1}
              max={65535}
              value={pf.guest_port}
              onChange={(e) => update(i, { guest_port: Number(e.target.value) })}
            />
            <select
              value={pf.protocol}
              onChange={(e) => update(i, { protocol: e.target.value })}
            >
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
            </select>
            <button type="button" className="pf-del" onClick={() => remove(i)}>
              ✕
            </button>
          </div>
        ))}
        <button type="button" className="pf-add" onClick={add}>
          + Add forward
        </button>
      </div>
    </label>
  );
}
