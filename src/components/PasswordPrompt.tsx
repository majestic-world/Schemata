import { useWorkspace } from "../store/workspace";
import { useState } from "react";
import { Dialog } from "./Dialog";
import { useConnections } from "../store/connections";
import { openConnection } from "../lib/connect";

export function PasswordPrompt() {
  const { passwordFor, askPassword } = useConnections();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  if (!passwordFor) return null;
  const close = () => {
    setPassword("");
    askPassword(null);
  };
  const submit = async () => {
    setBusy(true);
    const config = passwordFor;
    close();
    if (await openConnection(config, password)) useWorkspace.getState().setExpanded(`c:${config.id}`, true);
    setBusy(false);
  };
  return (
    <Dialog
      open
      onClose={close}
      title={`Connect to ${passwordFor.name}`}
      width={380}
      footer={
        <>
          <button className="btn" onClick={close}>Cancel</button>
          <button className="btn primary" disabled={busy} onClick={submit}>Connect</button>
        </>
      }
    >
      <form onSubmit={(e) => { e.preventDefault(); void submit(); }}>
        <label className="field">
          <span>Password for {passwordFor.user ?? "user"}</span>
          <input type="password" autoFocus value={password} onChange={(e) => setPassword(e.target.value)} />
        </label>
      </form>
    </Dialog>
  );
}
