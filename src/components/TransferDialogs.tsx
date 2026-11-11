import { useEffect, useRef, useState } from "react";
import { useConnection } from "../lib/useConnection";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { AlertCircle, CheckCircle2, FolderOpen } from "lucide-react";
import { api } from "../api/client";
import type { AppError, ColumnGenerator, CsvPreview, CsvReadOptions, ExportFormat, GeneratorKind, OperationProgress, TextEncoding } from "../api/types";
import { useTransfer, type TransferRequest } from "../store/transfer";
import { Dialog } from "./Dialog";
import { formatCount, plural } from "../lib/format";
import { assign, autoMap, toMapping, type MappingDraft } from "../lib/importMapping";
import { displayCell } from "../lib/cells";

type Phase<R> = { state: "form" } | { state: "running"; done: number; total: number | null } | { state: "done"; report: R } | { state: "failed"; error: AppError };

/** Runs a long operation, tracking its progress events; returns controls for the dialog. */
function useOperation<R>() {
  const [phase, setPhase] = useState<Phase<R>>({ state: "form" });
  const opId = useRef<string | null>(null);
  const run = async (start: (operationId: string) => Promise<R>) => {
    const id = crypto.randomUUID();
    opId.current = id;
    setPhase({ state: "running", done: 0, total: null });
    const unlisten = await listen<OperationProgress>("operation-progress", (e) => {
      if (e.payload.operationId === id) setPhase({ state: "running", done: e.payload.done, total: e.payload.total });
    });
    try {
      setPhase({ state: "done", report: await start(id) });
    } catch (e) {
      setPhase({ state: "failed", error: e as AppError });
    } finally {
      unlisten();
      opId.current = null;
    }
  };
  const cancel = () => opId.current && void api.cancelOperation(opId.current);
  return { phase, run, cancel, reset: () => setPhase({ state: "form" }) };
}

function ProgressView({ done, total, unit }: { done: number; total: number | null; unit: string }) {
  const pct = total ? Math.min(100, Math.round((done / Math.max(total, 1)) * 100)) : null;
  return (
    <div className="progress-view">
      <div className="progress-bar"><div className={pct === null ? "indeterminate" : ""} style={pct === null ? undefined : { width: `${pct}%` }} /></div>
      <span>{formatCount(done)}{total !== null ? ` of ${formatCount(total)}` : ""} {unit}{pct !== null ? ` (${pct}%)` : ""}</span>
    </div>
  );
}

function Outcome({ ok, title, children }: { ok: boolean; title: string; children?: React.ReactNode }) {
  return (
    <div className={`test-result ${ok ? "ok" : "fail"}`}>
      {ok ? <CheckCircle2 size={16} /> : <AlertCircle size={16} />}
      <div className="outcome-text"><b>{title}</b>{children}</div>
    </div>
  );
}

const pickSqlFile = async (): Promise<string | null> => {
  const p = await openDialog({ multiple: false, directory: false, filters: [{ name: "SQL", extensions: ["sql"] }] });
  return typeof p === "string" ? p : null;
};

const ENCODINGS: { value: TextEncoding; label: string }[] = [
  { value: "utf8", label: "UTF-8" },
  { value: "utf8Bom", label: "UTF-8 with BOM" },
  { value: "windows1252", label: "Windows-1252 (Latin)" },
];

export function TransferDialogs() {
  const request = useTransfer((s) => s.request);
  if (!request) return null;
  switch (request.kind) {
    case "export":
      return <ExportDialog key={JSON.stringify(request)} req={request} />;
    case "import":
      return <ImportDialog key={JSON.stringify(request)} req={request} />;
    case "runSql":
      return <RunSqlDialog key={JSON.stringify(request)} req={request} />;
    case "backup":
      return <BackupDialog key={JSON.stringify(request)} req={request} />;
    case "restore":
      return <RestoreDialog key={JSON.stringify(request)} req={request} />;
    case "copyTable":
      return <CopyTableDialog key={JSON.stringify(request)} req={request} />;
    case "generate":
      return <GenerateDialog key={JSON.stringify(request)} req={request} />;
    default: {
      const never: never = request;
      return never;
    }
  }
}

function ExportDialog({ req }: { req: Extract<TransferRequest, { kind: "export" }> }) {
  const close = () => useTransfer.getState().open(null);
  const [format, setFormat] = useState<"csv" | "json" | "sql">("csv");
  const [delimiter, setDelimiter] = useState(",");
  const [header, setHeader] = useState(true);
  const [encoding, setEncoding] = useState<TextEncoding>("utf8");
  const [nullText, setNullText] = useState("");
  const [quote, setQuote] = useState('"');
  const [includeCreate, setIncludeCreate] = useState(true);
  const [tableName, setTableName] = useState(req.source.kind === "table" ? req.source.table : "query_result");
  const op = useOperation<{ rows: number }>();

  const start = async () => {
    const ext = format === "csv" ? "csv" : format === "json" ? "json" : "sql";
    const path = await saveDialog({ defaultPath: `${req.label}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
    if (typeof path !== "string") return;
    const fmt: ExportFormat =
      format === "csv"
        ? { format: "csv", options: { delimiter: delimiter === "\\t" ? "\t" : delimiter, quote, header, encoding, nullText } }
        : format === "json"
          ? { format: "json" }
          : { format: "sql", options: { includeCreate: includeCreate && req.source.kind === "table", tableName } };
    await op.run((id) => api.exportData(req.connectionId, req.database, req.source, fmt, path, id));
  };

  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title={`Export ${req.label}`}
      width={500}
      footer={
        op.phase.state === "running" ? (
          <button className="btn danger" onClick={op.cancel}>Cancel</button>
        ) : op.phase.state === "form" ? (
          <><button className="btn" onClick={close}>Cancel</button><button className="btn primary" onClick={start}>Export…</button></>
        ) : (
          <button className="btn primary" onClick={close}>Close</button>
        )
      }
    >
      {op.phase.state === "form" && (
        <div className="form">
          <div className="segmented engine-picker">
            {(["csv", "json", "sql"] as const).map((f) => <button key={f} className={format === f ? "on" : ""} onClick={() => setFormat(f)}>{f.toUpperCase()}</button>)}
          </div>
          {format === "csv" && (
            <>
              <div className="row">
                <label className="field grow"><span>Delimiter</span>
                  <select value={delimiter} onChange={(e) => setDelimiter(e.target.value)}>
                    <option value=",">Comma (,)</option><option value=";">Semicolon (;)</option><option value="\t">Tab</option><option value="|">Pipe (|)</option>
                  </select>
                </label>
                <label className="field"><span>Quote</span>
                  <select value={quote} onChange={(e) => setQuote(e.target.value)}>
                    <option value={'"'}>Double (")</option><option value="'">Single (')</option>
                  </select>
                </label>
                <label className="field grow"><span>Encoding</span>
                  <select value={encoding} onChange={(e) => setEncoding(e.target.value as TextEncoding)}>{ENCODINGS.map((x) => <option key={x.value} value={x.value}>{x.label}</option>)}</select>
                </label>
              </div>
              <label className="field"><span>Write NULL as</span><input value={nullText} placeholder="(empty)" onChange={(e) => setNullText(e.target.value)} /></label>
              <label className="check"><input type="checkbox" checked={header} onChange={(e) => setHeader(e.target.checked)} />Include a header row</label>
            </>
          )}
          {format === "json" && <p className="hint">An array of objects, one per row. Large integers and decimals are written as strings to keep their precision.</p>}
          {format === "sql" && (
            <>
              <label className="field"><span>Table name in INSERT statements</span><input value={tableName} onChange={(e) => setTableName(e.target.value)} /></label>
              {req.source.kind === "table" && <label className="check"><input type="checkbox" checked={includeCreate} onChange={(e) => setIncludeCreate(e.target.checked)} />Include CREATE TABLE</label>}
            </>
          )}
        </div>
      )}
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="rows" />}
      {op.phase.state === "done" && <Outcome ok title={`Exported ${plural(op.phase.report.rows, "row")}`} />}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Export cancelled" : "Export failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}

function ImportDialog({ req }: { req: Extract<TransferRequest, { kind: "import" }> }) {
  const qc = useQueryClient();
  const close = () => useTransfer.getState().open(null);
  const [path, setPath] = useState("");
  const [options, setOptions] = useState<CsvReadOptions>({ delimiter: ",", quote: '"', hasHeader: true, encoding: "utf8" });
  const [preview, setPreview] = useState<CsvPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [draft, setDraft] = useState<MappingDraft>([]);
  const [emptyAsNull, setEmptyAsNull] = useState(true);
  const [skipErrors, setSkipErrors] = useState(false);
  const info = useQuery({ queryKey: ["tableInfo", req.connectionId, req.database, req.table], queryFn: () => api.tableInfo(req.connectionId, req.database, req.table) });
  const targets = info.data?.columns.map((c) => c.name) ?? [];
  const op = useOperation<Awaited<ReturnType<typeof api.importCsv>>>();

  useEffect(() => {
    if (!path) return;
    api
      .previewCsv(path, options)
      .then((p) => {
        setPreview(p);
        setPreviewError(null);
        setDraft(autoMap(p.headers, targets));
      })
      .catch((e) => {
        setPreview(null);
        setPreviewError((e as AppError).message);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, options, info.data]);

  const pick = async () => {
    const p = await openDialog({ multiple: false, directory: false, filters: [{ name: "CSV", extensions: ["csv", "txt", "tsv"] }] });
    if (typeof p === "string") setPath(p);
  };
  const start = () =>
    op.run(async (id) => {
      const report = await api.importCsv(req.connectionId, req.database, req.table, path, { options, mapping: toMapping(draft), emptyAsNull, skipErrors }, id);
      void qc.invalidateQueries({ queryKey: ["page", req.connectionId, req.database, req.table] });
      void qc.invalidateQueries({ queryKey: ["count", req.connectionId, req.database, req.table] });
      void qc.invalidateQueries({ queryKey: ["objects", req.connectionId, req.database] });
      return report;
    });
  const mapped = toMapping(draft).length;

  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title={`Import CSV into ${req.table}`}
      width={720}
      footer={
        op.phase.state === "running" ? (
          <button className="btn danger" onClick={op.cancel}>Cancel</button>
        ) : op.phase.state === "form" ? (
          <><button className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={!preview || mapped === 0} onClick={start}>Import</button></>
        ) : (
          <button className="btn primary" onClick={close}>Close</button>
        )
      }
    >
      {op.phase.state === "form" && (
        <div className="form">
          <div className="row">
            <label className="field grow"><span>CSV file</span><input value={path} onChange={(e) => setPath(e.target.value)} aria-label="CSV file" /></label>
            <button className="btn" onClick={pick}><FolderOpen size={15} />Browse…</button>
          </div>
          <div className="row">
            <label className="field grow"><span>Delimiter</span>
              <select value={options.delimiter} aria-label="Delimiter" onChange={(e) => setOptions({ ...options, delimiter: e.target.value })}>
                <option value=",">Comma (,)</option><option value=";">Semicolon (;)</option><option value={"\t"}>Tab</option><option value="|">Pipe (|)</option>
              </select>
            </label>
            <label className="field grow"><span>Encoding</span>
              <select value={options.encoding} onChange={(e) => setOptions({ ...options, encoding: e.target.value as TextEncoding })}>
                {ENCODINGS.filter((x) => x.value !== "utf8Bom").map((x) => <option key={x.value} value={x.value}>{x.label}</option>)}
              </select>
            </label>
            <label className="check"><input type="checkbox" checked={options.hasHeader} onChange={(e) => setOptions({ ...options, hasHeader: e.target.checked })} />First row is a header</label>
          </div>
          {previewError && <p className="error-text">{previewError}</p>}
          {preview && (
            <div className="mapping">
              <table className="list-table">
                <thead>
                  <tr>{preview.headers.map((h, i) => (
                    <th key={i}>
                      <div className="map-head">
                        <span className="muted">{h}</span>
                        <select className="input sm" aria-label={`Target for ${h}`} value={draft[i] ?? ""} onChange={(e) => setDraft((d) => assign(d, i, e.target.value || null))}>
                          <option value="">(skip)</option>
                          {targets.map((t) => <option key={t} value={t}>{t}</option>)}
                        </select>
                      </div>
                    </th>
                  ))}</tr>
                </thead>
                <tbody>
                  {preview.rows.slice(0, 8).map((r, i) => <tr key={i}>{preview.headers.map((_, j) => <td key={j}>{r[j] ?? ""}</td>)}</tr>)}
                </tbody>
              </table>
            </div>
          )}
          <label className="check"><input type="checkbox" checked={emptyAsNull} onChange={(e) => setEmptyAsNull(e.target.checked)} />Import empty fields as NULL</label>
          <label className="check"><input type="checkbox" checked={skipErrors} onChange={(e) => setSkipErrors(e.target.checked)} />Skip rows that fail (otherwise the first failure rolls back the whole import)</label>
        </div>
      )}
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="rows read" />}
      {op.phase.state === "done" && (
        <Outcome ok={op.phase.report.failedCount === 0} title={`${plural(op.phase.report.inserted, "row")} imported${op.phase.report.failedCount ? `, ${formatCount(op.phase.report.failedCount)} failed` : ""}`}>
          {op.phase.report.failedCount > 0 && !skipErrors && <span>The import was rolled back.</span>}
          {op.phase.report.failed.map((f) => <span key={f.line}>Line {f.line}: {f.message}</span>)}
        </Outcome>
      )}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Import cancelled and rolled back" : "Import failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}

function RunSqlDialog({ req }: { req: Extract<TransferRequest, { kind: "runSql" }> }) {
  const qc = useQueryClient();
  const close = () => useTransfer.getState().open(null);
  const [path, setPath] = useState("");
  const [stopOnError, setStopOnError] = useState(true);
  const op = useOperation<Awaited<ReturnType<typeof api.runSqlFile>>>();
  const pick = async () => {
    const p = await pickSqlFile();
    if (p) setPath(p);
  };
  const start = () =>
    op.run(async (id) => {
      const report = await api.runSqlFile(req.connectionId, req.database, path, stopOnError, id);
      void qc.invalidateQueries({ queryKey: ["objects", req.connectionId] });
      void qc.invalidateQueries({ queryKey: ["databases", req.connectionId] });
      return report;
    });
  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title={`Run SQL file on ${req.database}`}
      width={560}
      footer={
        op.phase.state === "running" ? (
          <button className="btn danger" onClick={op.cancel}>Cancel</button>
        ) : op.phase.state === "form" ? (
          <><button className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={!path} onClick={start}>Run</button></>
        ) : (
          <button className="btn primary" onClick={close}>Close</button>
        )
      }
    >
      {op.phase.state === "form" && (
        <div className="form">
          <div className="row">
            <label className="field grow"><span>SQL file</span><input value={path} onChange={(e) => setPath(e.target.value)} aria-label="SQL file" /></label>
            <button className="btn" onClick={pick}><FolderOpen size={15} />Browse…</button>
          </div>
          <label className="check"><input type="checkbox" checked={stopOnError} onChange={(e) => setStopOnError(e.target.checked)} />Stop at the first error</label>
        </div>
      )}
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="bytes read" />}
      {op.phase.state === "done" && (
        <Outcome ok={op.phase.report.failed.length === 0} title={`${plural(op.phase.report.executed, "statement")} executed${op.phase.report.failed.length ? `, ${op.phase.report.failed.length} failed` : ""}`}>
          {op.phase.report.failed.slice(0, 20).map((f) => <span key={f.index}>#{f.index + 1}: {f.message} — <code>{f.statement}</code></span>)}
        </Outcome>
      )}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Stopped" : "Failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}

function BackupDialog({ req }: { req: Extract<TransferRequest, { kind: "backup" }> }) {
  const close = () => useTransfer.getState().open(null);
  const [structure, setStructure] = useState(true);
  const [data, setData] = useState(req.data);
  const op = useOperation<Awaited<ReturnType<typeof api.backupDatabase>>>();
  const start = async () => {
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const path = await saveDialog({ defaultPath: `${req.database}-${stamp}.sql`, filters: [{ name: "SQL", extensions: ["sql"] }] });
    if (typeof path !== "string") return;
    await op.run((id) => api.backupDatabase(req.connectionId, req.database, path, { structure, data }, id));
  };
  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title={`Back up ${req.database}`}
      width={460}
      footer={
        op.phase.state === "running" ? (
          <button className="btn danger" onClick={op.cancel}>Cancel</button>
        ) : op.phase.state === "form" ? (
          <><button className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={!structure && !data} onClick={start}>Back Up…</button></>
        ) : (
          <button className="btn primary" onClick={close}>Close</button>
        )
      }
    >
      {op.phase.state === "form" && (
        <div className="form">
          <p className="dialog-text">Writes a .sql file with tables, views, indexes, foreign keys and triggers that can be restored here or with the database's own tools.</p>
          <label className="check"><input type="checkbox" checked={structure} onChange={(e) => setStructure(e.target.checked)} />Structure (CREATE statements)</label>
          <label className="check"><input type="checkbox" checked={data} onChange={(e) => setData(e.target.checked)} />Data (INSERT statements)</label>
        </div>
      )}
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="rows" />}
      {op.phase.state === "done" && <Outcome ok title={`Backed up ${plural(op.phase.report.tables, "table")} and ${plural(op.phase.report.rows, "row")}`} />}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Backup cancelled" : "Backup failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}

function RestoreDialog({ req }: { req: Extract<TransferRequest, { kind: "restore" }> }) {
  const qc = useQueryClient();
  const close = () => useTransfer.getState().open(null);
  const conn = useConnection(req.connectionId);
  const sqlite = conn?.engine === "sqlite";
  const databases = useQuery({ queryKey: ["databases", req.connectionId], queryFn: () => api.listDatabases(req.connectionId) });
  const [path, setPath] = useState("");
  const [target, setTarget] = useState(req.database ?? "");
  useEffect(() => {
    if (sqlite && !target) setTarget("main");
  }, [sqlite, target]);
  const [newName, setNewName] = useState("");
  const createNew = !sqlite && target === "";
  const op = useOperation<Awaited<ReturnType<typeof api.runSqlFile>>>();
  const pick = async () => {
    const p = await pickSqlFile();
    if (p) setPath(p);
  };
  const start = () =>
    op.run(async (id) => {
      const db = createNew ? newName.trim() : target;
      if (createNew) {
        await api.createDatabase(req.connectionId, { name: db, charset: "utf8mb4", collation: null });
        void qc.invalidateQueries({ queryKey: ["databases", req.connectionId] });
      }
      const report = await api.runSqlFile(req.connectionId, db, path, true, id);
      void qc.invalidateQueries({ queryKey: ["objects", req.connectionId] });
      return report;
    });
  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title="Restore backup"
      width={540}
      footer={
        op.phase.state === "running" ? (
          <button className="btn danger" onClick={op.cancel}>Cancel</button>
        ) : op.phase.state === "form" ? (
          <><button className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={!path || (createNew ? !newName.trim() : !target)} onClick={start}>Restore</button></>
        ) : (
          <button className="btn primary" onClick={close}>Close</button>
        )
      }
    >
      {op.phase.state === "form" && (
        <div className="form">
          <div className="row">
            <label className="field grow"><span>Backup file (.sql)</span><input value={path} onChange={(e) => setPath(e.target.value)} aria-label="Backup file" /></label>
            <button className="btn" onClick={pick}><FolderOpen size={15} />Browse…</button>
          </div>
          <label className="field"><span>Restore into</span>
            <select value={target} aria-label="Target database" onChange={(e) => setTarget(e.target.value)}>
              {!sqlite && <option value="">New database…</option>}
              {databases.data?.map((d) => <option key={d.name} value={d.name}>{d.name}</option>)}
            </select>
          </label>
          {createNew && <label className="field"><span>New database name</span><input value={newName} aria-label="New database name" onChange={(e) => setNewName(e.target.value)} /></label>}
          {!createNew && <p className="hint">Tables in the backup replace tables with the same name in the target.</p>}
        </div>
      )}
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="bytes read" />}
      {op.phase.state === "done" && (
        <Outcome ok={op.phase.report.failed.length === 0} title={op.phase.report.failed.length ? "Restore stopped at an error" : `Restored (${plural(op.phase.report.executed, "statement")})`}>
          {op.phase.report.failed.map((f) => <span key={f.index}>#{f.index + 1}: {f.message} — <code>{f.statement}</code></span>)}
        </Outcome>
      )}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Restore cancelled" : "Restore failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}

/** Free name for a copy: the source name, or `name_copy`, `name_copy2`, … when it is taken. */
function copyName(name: string, existing: string[]): string {
  const taken = new Set(existing.map((n) => n.toLowerCase()));
  if (!taken.has(name.toLowerCase())) return name;
  for (let i = 1; ; i++) {
    const candidate = `${name}_copy${i === 1 ? "" : i}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

/** Pasting a copied table: starts right away (like Navicat) and reports progress and the outcome. */
function CopyTableDialog({ req }: { req: Extract<TransferRequest, { kind: "copyTable" }> }) {
  const close = () => useTransfer.getState().open(null);
  const qc = useQueryClient();
  const { source, target } = req;
  const [name, setName] = useState<string | null>(null);
  const op = useOperation<Awaited<ReturnType<typeof api.copyTable>>>();
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void (async () => {
      const objects = await api.listObjects(target.connectionId, target.database).catch(() => []);
      const to = copyName(source.name, objects.map((o) => o.name));
      setName(to);
      await op.run((id) => api.copyTable(source.connectionId, source.database, source.name, target.connectionId, { db: target.database, name: to, data: true }, id));
      void qc.invalidateQueries({ queryKey: ["objects", target.connectionId, target.database] });
    })();
  }, [op, qc, source, target]);
  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title={`Copy ${source.name} to ${target.database}`}
      width={460}
      footer={op.phase.state === "running" || op.phase.state === "form" ? <button className="btn danger" onClick={op.cancel}>Cancel</button> : <button className="btn primary" onClick={close}>Close</button>}
    >
      <p className="dialog-text">
        {source.database}.{source.name} → {target.database}.{name ?? "…"}
      </p>
      {(op.phase.state === "running" || op.phase.state === "form") && <ProgressView done={op.phase.state === "running" ? op.phase.done : 0} total={op.phase.state === "running" ? op.phase.total : null} unit="rows" />}
      {op.phase.state === "done" && <Outcome ok title={`Copied ${source.name} with ${plural(op.phase.report.rows, "row")}`}>{name !== source.name && <span>Saved as {name} because {source.name} already exists there.</span>}</Outcome>}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Copy cancelled" : "Copy failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}

const GENERATOR_GROUPS: { label: string; kinds: GeneratorKind[] }[] = [
  { label: "Basic", kinds: ["skip", "null", "sequence", "integer", "decimal", "boolean"] },
  { label: "People", kinds: ["firstName", "lastName", "fullName", "email", "username", "phone"] },
  { label: "Places", kinds: ["address", "city", "state", "country", "zipCode"] },
  { label: "Business & web", kinds: ["company", "title", "code", "url", "ipAddress", "uuid", "hash", "color", "slug"] },
  { label: "Text (lorem ipsum)", kinds: ["word", "sentence", "paragraph"] },
  { label: "Date & time", kinds: ["date", "dateTime", "time", "year"] },
  { label: "Other", kinds: ["enumValue", "json", "bytes", "foreignKey"] },
];
const GENERATOR_LABEL: Record<GeneratorKind, string> = {
  skip: "Database default (skip)",
  null: "NULL",
  sequence: "Sequence (after current max)",
  integer: "Integer",
  decimal: "Decimal",
  boolean: "Boolean (0/1)",
  firstName: "First name",
  lastName: "Last name",
  fullName: "Full name",
  email: "Email",
  username: "Username",
  phone: "Phone",
  company: "Company",
  city: "City",
  state: "State / province",
  country: "Country",
  address: "Street address",
  zipCode: "ZIP / postal code",
  url: "URL",
  uuid: "UUID",
  ipAddress: "IP address",
  word: "Word",
  title: "Product name",
  sentence: "Sentence",
  paragraph: "Paragraph",
  slug: "Slug",
  code: "Code (ABC-1234)",
  color: "Hex color",
  hash: "Hash (64 hex)",
  date: "Date",
  dateTime: "Date and time",
  time: "Time",
  year: "Year",
  enumValue: "Enum value",
  json: "JSON object",
  bytes: "Random bytes",
  foreignKey: "Existing referenced value",
};

/** Fills a table with realistic test data (Navicat's Data Generation): a generator per column, previewed first. */
function GenerateDialog({ req }: { req: Extract<TransferRequest, { kind: "generate" }> }) {
  const close = () => useTransfer.getState().open(null);
  const qc = useQueryClient();
  const { connectionId, database, table } = req;
  const [rows, setRows] = useState(100);
  const [emptyFirst, setEmptyFirst] = useState(false);
  const [columns, setColumns] = useState<ColumnGenerator[] | null>(null);
  const info = useQuery({ queryKey: ["tableInfo", connectionId, database, table], queryFn: () => api.tableInfo(connectionId, database, table) });
  const suggested = useQuery({ queryKey: ["generators", connectionId, database, table], queryFn: () => api.suggestGenerators(connectionId, database, table), staleTime: 0 });
  useEffect(() => {
    if (suggested.data && !columns) setColumns(suggested.data);
  }, [suggested.data, columns]);
  const preview = useQuery({
    queryKey: ["generatedPreview", connectionId, database, table, columns],
    queryFn: () => api.previewGenerated(connectionId, database, table, columns!),
    enabled: !!columns && columns.some((c) => c.kind !== "skip"),
    retry: false,
    staleTime: 0,
  });
  const op = useOperation<Awaited<ReturnType<typeof api.generateData>>>();
  const validRows = Number.isInteger(rows) && rows >= 1 && rows <= 1_000_000;
  const start = async () => {
    if (!columns) return;
    await op.run((id) => api.generateData(connectionId, database, table, { rows, columns, emptyFirst }, id));
    for (const key of [["page", connectionId, database, table], ["count", connectionId, database, table], ["objects", connectionId, database], ["tableInfo", connectionId, database, table]]) {
      void qc.invalidateQueries({ queryKey: key });
    }
  };
  const typeOf = (column: string) => info.data?.columns.find((c) => c.name === column)?.dataType ?? "";
  return (
    <Dialog
      open
      onClose={() => (op.phase.state === "running" ? op.cancel() : close())}
      title={`Generate data for ${table}`}
      width={780}
      footer={
        op.phase.state === "running" ? (
          <button className="btn danger" onClick={op.cancel}>Cancel</button>
        ) : op.phase.state === "form" ? (
          <>
            <button className="btn" onClick={close}>Cancel</button>
            <button className="btn primary" disabled={!columns || !validRows || preview.isError} onClick={start}>Generate {validRows ? plural(rows, "row") : "rows"}</button>
          </>
        ) : (
          <button className="btn primary" onClick={close}>Close</button>
        )
      }
    >
      {op.phase.state === "form" && (
        <div className="form">
          <div className="gen-options">
            <label className="field">
              <span>Rows</span>
              <input className="input" type="number" min={1} max={1_000_000} value={Number.isNaN(rows) ? "" : rows} onChange={(e) => setRows(e.target.valueAsNumber)} />
            </label>
            <label className="check"><input type="checkbox" checked={emptyFirst} onChange={(e) => setEmptyFirst(e.target.checked)} />Delete existing rows first</label>
          </div>
          {suggested.isError && <p className="error-text">{(suggested.error as { message: string }).message}</p>}
          {columns && (
            <div className="gen-columns">
              <table className="list-table">
                <thead><tr><th>Column</th><th>Type</th><th>Generator</th></tr></thead>
                <tbody>
                  {columns.map((c, i) => (
                    <tr key={c.column}>
                      <td>{c.column}</td>
                      <td className="muted">{typeOf(c.column)}</td>
                      <td>
                        <select className="input" aria-label={`Generator for ${c.column}`} value={c.kind} onChange={(e) => setColumns(columns.map((x, j) => (j === i ? { ...x, kind: e.target.value as GeneratorKind } : x)))}>
                          {GENERATOR_GROUPS.map((g) => (
                            <optgroup key={g.label} label={g.label}>
                              {g.kinds.map((k) => <option key={k} value={k}>{GENERATOR_LABEL[k]}</option>)}
                            </optgroup>
                          ))}
                        </select>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <div className="gen-preview-head">
            <span className="hint">Preview</span>
            <button className="btn sm" disabled={!preview.data && !preview.isError} onClick={() => void preview.refetch()}>Shuffle</button>
          </div>
          {preview.isError ? (
            <p className="error-text">{(preview.error as { message: string }).message}</p>
          ) : (
            <div className="gen-preview">
              <table className="list-table">
                <thead><tr>{preview.data?.columns.map((c) => <th key={c.name}>{c.name}</th>)}</tr></thead>
                <tbody>
                  {preview.data?.rows.map((r, i) => (
                    <tr key={i}>{r.map((v, j) => { const d = displayCell(v); return <td key={j} className={d.tone === "null" ? "muted" : undefined}>{d.text}</td>; })}</tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="rows" />}
      {op.phase.state === "done" && <Outcome ok title={`Inserted ${plural(op.phase.report.rows, "row")} into ${table}`} />}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Generation cancelled — nothing was inserted" : "Generation failed — nothing was inserted"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}
