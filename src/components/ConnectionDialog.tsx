import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, AlertCircle, FolderOpen, FilePlus2 } from "lucide-react";
import { Dialog } from "./Dialog";
import { EngineIcon } from "./icons";
import { api } from "../api/client";
import { ENGINE_LABEL, type AppError, type ConnectParams, type ConnectionConfig, type Engine } from "../api/types";
import { useConnections } from "../store/connections";
import { notify, notifyError } from "../store/toasts";

const COLORS = [null, "#14a3a3", "#2f7fc1", "#8b5cf6", "#e04f55", "#e8912d", "#2e9e5b"];
const SQLITE_FILTERS = [{ name: "SQLite database", extensions: ["db", "sqlite", "sqlite3", "db3"] }];
const DEFAULT_PORT: Record<Engine, number | null> = { mySql: 3306, mariaDb: 3306, sqlite: null };

function blank(): ConnectionConfig {
  return {
    id: crypto.randomUUID(),
    name: "",
    engine: "mySql",
    color: null,
    host: "localhost",
    port: 3306,
    user: "root",
    database: null,
    filePath: null,
    savePassword: true,
  };
}

type TestState = { ok: true; version: string } | { ok: false; error: AppError } | null;

export function ConnectionDialog() {
  const { dialog, showDialog } = useConnections();
  const qc = useQueryClient();
  const [config, setConfig] = useState<ConnectionConfig>(blank);
  const [password, setPassword] = useState("");
  const [passwordTouched, setPasswordTouched] = useState(false);
  const [createFile, setCreateFile] = useState(false);
  const [test, setTest] = useState<TestState>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!dialog) return;
    setConfig(dialog.mode === "edit" ? dialog.config : blank());
    setPassword("");
    setPasswordTouched(false);
    setCreateFile(false);
    setTest(null);
  }, [dialog]);

  if (!dialog) return null;
  const editing = dialog.mode === "edit";
  const sqlite = config.engine === "sqlite";
  const patch = (p: Partial<ConnectionConfig>) => {
    setConfig((c) => ({ ...c, ...p }));
    setTest(null);
  };

  /** Password to use: the typed one, or (when editing without touching it) the stored one on the backend. */
  const typedPassword = passwordTouched || !editing ? password : null;

  const params = async (): Promise<ConnectParams> => ({
    engine: config.engine,
    host: config.host,
    port: config.port,
    user: config.user,
    password: typedPassword,
    database: config.database || null,
    filePath: config.filePath,
    createFile: sqlite && createFile,
  });

  const runTest = async () => {
    setBusy(true);
    try {
      setTest({ ok: true, version: await api.testConnection(await params(), editing ? config.id : null) });
    } catch (e) {
      setTest({ ok: false, error: e as AppError });
    } finally {
      setBusy(false);
    }
  };

  const valid = config.name.trim() !== "" && (sqlite ? !!config.filePath : !!config.host && !!config.user);

  const submit = async () => {
    setBusy(true);
    try {
      if (sqlite && createFile) await api.testConnection(await params(), null);
      const cleaned: ConnectionConfig = sqlite
        ? { ...config, host: null, port: null, user: null, database: null, savePassword: false }
        : { ...config, filePath: null };
      const outcome = await api.saveConnection({ ...cleaned, name: config.name.trim() }, sqlite ? null : typedPassword);
      if (!sqlite && config.savePassword && typedPassword !== null && !outcome.passwordSaved) {
        notifyError("Password not saved", { message: "The system credential store is unavailable; you will be asked for the password when connecting." });
      }
      await qc.invalidateQueries({ queryKey: ["connections"] });
      notify(editing ? "Connection updated" : "Connection created", outcome.config.name);
      showDialog(null);
    } catch (e) {
      notifyError("Could not save connection", e);
    } finally {
      setBusy(false);
    }
  };

  const pickFile = async (create: boolean) => {
    const path = create
      ? await saveFile({ filters: SQLITE_FILTERS, defaultPath: "database.db" })
      : await openFile({ filters: SQLITE_FILTERS, multiple: false, directory: false });
    if (typeof path === "string") {
      patch({ filePath: path, name: config.name || path.split(/[\\/]/).pop()!.replace(/\.[^.]+$/, "") });
      setCreateFile(create);
    }
  };

  return (
    <Dialog
      open
      onClose={() => showDialog(null)}
      title={editing ? "Edit Connection" : "New Connection"}
      width={540}
      className="conn-dialog"
      footer={
        <>
          <button className="btn" disabled={busy || (sqlite ? !config.filePath : !config.host)} onClick={runTest}>Test Connection</button>
          <div className="grow" />
          <button className="btn" onClick={() => showDialog(null)}>Cancel</button>
          <button className="btn primary" disabled={busy || !valid} onClick={submit}>Save</button>
        </>
      }
    >
      <div className="conn-form">
        {!editing && (
          <div className="segmented engine-picker span-all" role="radiogroup" aria-label="Database engine">
            {(Object.keys(ENGINE_LABEL) as Engine[]).map((e) => (
              <button key={e} type="button" role="radio" aria-checked={config.engine === e} className={config.engine === e ? "on" : ""} onClick={() => patch({ engine: e, port: DEFAULT_PORT[e] })}>
                <EngineIcon engine={e} size={16} />{ENGINE_LABEL[e]}
              </button>
            ))}
          </div>
        )}
        <label className="field span-all">
          <span>Connection name</span>
          <input autoFocus value={config.name} onChange={(e) => patch({ name: e.target.value })} placeholder={sqlite ? "Local database" : "Production"} />
        </label>
        <div className="field span-all">
          <span>Color</span>
          <div className="swatches" role="radiogroup" aria-label="Color">
            {COLORS.map((c) => (
              <button
                key={c ?? "none"}
                type="button"
                role="radio"
                aria-checked={config.color === c}
                aria-label={c ?? "No color"}
                title={c ?? "No color"}
                className={`swatch${config.color === c ? " on" : ""}${c ? "" : " none"}`}
                style={c ? { background: c } : undefined}
                onClick={() => patch({ color: c })}
              />
            ))}
          </div>
        </div>
        {sqlite ? (
          <>
            <div className="conn-section">File</div>
            <label className="field span-all">
              <span>Database file</span>
              <input value={config.filePath ?? ""} onChange={(e) => { patch({ filePath: e.target.value || null }); setCreateFile(false); }} placeholder="C:\data\app.db" />
            </label>
            <div className="conn-actions span-all">
              <button type="button" className="btn" onClick={() => pickFile(false)}><FolderOpen size={15} />Open existing…</button>
              <button type="button" className="btn" onClick={() => pickFile(true)}><FilePlus2 size={15} />Create new…</button>
              {createFile && <span className="hint">A new file will be created.</span>}
            </div>
          </>
        ) : (
          <>
            <div className="conn-section">Server</div>
            <label className="field span-2">
              <span>Host</span>
              <input value={config.host ?? ""} onChange={(e) => patch({ host: e.target.value })} placeholder="localhost" />
            </label>
            <label className="field">
              <span>Port</span>
              <input type="number" value={config.port ?? ""} onChange={(e) => patch({ port: e.target.value ? Number(e.target.value) : null })} />
            </label>
            <div className="conn-section">Authentication</div>
            <label className="field">
              <span>User</span>
              <input value={config.user ?? ""} onChange={(e) => patch({ user: e.target.value })} />
            </label>
            <label className="field span-2">
              <span>Password</span>
              <input
                type="password"
                value={password}
                placeholder={editing && config.savePassword && !passwordTouched ? "Saved — leave blank to keep" : ""}
                onChange={(e) => { setPassword(e.target.value); setPasswordTouched(true); setTest(null); }}
              />
            </label>
            <label className="check span-all">
              <input type="checkbox" checked={config.savePassword} onChange={(e) => patch({ savePassword: e.target.checked })} />
              Save password in the system credential store
            </label>
            <label className="field span-all">
              <span>Default database <em className="optional">optional</em></span>
              <input value={config.database ?? ""} onChange={(e) => patch({ database: e.target.value || null })} />
            </label>
          </>
        )}
        {test && (
          <div className={`test-result span-all ${test.ok ? "ok" : "fail"}`}>
            {test.ok ? <CheckCircle2 size={16} /> : <AlertCircle size={16} />}
            <span>{test.ok ? `Connected — ${test.version}` : test.error.message}</span>
          </div>
        )}
      </div>
    </Dialog>
  );
}
