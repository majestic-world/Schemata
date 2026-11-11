import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { Dialog } from "./Dialog";
import { api } from "../api/client";
import { useDialogs } from "../store/dialogs";
import { notify, notifyError } from "../store/toasts";

export function CreateDatabaseDialog() {
  const connectionId = useDialogs((s) => s.createDatabaseFor);
  const set = useDialogs((s) => s.set);
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [charset, setCharset] = useState("utf8mb4");
  const [collation, setCollation] = useState("");
  const [busy, setBusy] = useState(false);
  const charsets = useQuery({
    queryKey: ["charsets", connectionId],
    queryFn: () => api.listCharsets(connectionId!),
    enabled: !!connectionId,
    staleTime: Infinity,
  });
  useEffect(() => {
    setName("");
    setCharset("utf8mb4");
    setCollation("");
  }, [connectionId]);
  if (!connectionId) return null;
  const current = charsets.data?.find((c) => c.name === charset);
  const close = () => set({ createDatabaseFor: null });
  const submit = async () => {
    setBusy(true);
    try {
      await api.createDatabase(connectionId, { name: name.trim(), charset, collation: collation || current?.defaultCollation || null });
      await qc.invalidateQueries({ queryKey: ["databases", connectionId] });
      notify("Database created", name.trim());
      close();
    } catch (e) {
      notifyError("Could not create database", e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      onClose={close}
      title="New Database"
      width={440}
      footer={<><button className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={busy || !name.trim()} onClick={submit}>Create</button></>}
    >
      <div className="form">
        <label className="field"><span>Database name</span><input autoFocus value={name} onChange={(e) => setName(e.target.value)} /></label>
        <div className="row">
          <label className="field grow">
            <span>Character set</span>
            <select value={charset} onChange={(e) => { setCharset(e.target.value); setCollation(""); }}>
              {charsets.data?.map((c) => <option key={c.name} value={c.name}>{c.name}</option>)}
            </select>
          </label>
          <label className="field grow">
            <span>Collation</span>
            <select value={collation} onChange={(e) => setCollation(e.target.value)}>
              <option value="">Default ({current?.defaultCollation ?? "—"})</option>
              {current?.collations.map((c) => <option key={c} value={c}>{c}</option>)}
            </select>
          </label>
        </div>
      </div>
    </Dialog>
  );
}

export function AttachDatabaseDialog() {
  const connectionId = useDialogs((s) => s.attachDatabaseFor);
  const set = useDialogs((s) => s.set);
  const qc = useQueryClient();
  const [path, setPath] = useState("");
  const [alias, setAlias] = useState("");
  useEffect(() => {
    setPath("");
    setAlias("");
  }, [connectionId]);
  if (!connectionId) return null;
  const close = () => set({ attachDatabaseFor: null });
  const pick = async () => {
    const file = await openFile({ multiple: false, directory: false, filters: [{ name: "SQLite database", extensions: ["db", "sqlite", "sqlite3", "db3"] }] });
    if (typeof file === "string") {
      setPath(file);
      if (!alias) setAlias(file.split(/[\\/]/).pop()!.replace(/\.[^.]+$/, "").replace(/\W+/g, "_"));
    }
  };
  const submit = async () => {
    try {
      await api.attachDatabase(connectionId, path, alias.trim());
      await qc.invalidateQueries({ queryKey: ["databases", connectionId] });
      notify("Database attached", alias.trim());
      close();
    } catch (e) {
      notifyError("Could not attach database", e);
    }
  };
  return (
    <Dialog
      open
      onClose={close}
      title="Attach Database"
      width={480}
      footer={<><button className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={!path || !alias.trim()} onClick={submit}>Attach</button></>}
    >
      <div className="form">
        <div className="row">
          <label className="field grow"><span>Database file</span><input value={path} onChange={(e) => setPath(e.target.value)} /></label>
          <button className="btn" onClick={pick}><FolderOpen size={15} />Browse…</button>
        </div>
        <label className="field"><span>Alias</span><input value={alias} onChange={(e) => setAlias(e.target.value)} /></label>
      </div>
    </Dialog>
  );
}
