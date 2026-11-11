import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { AlertCircle, CheckCircle2, FolderOpen } from "lucide-react";
import { api } from "../api/client";
import type { AppError, CsvPreview, CsvReadOptions, ExportFormat, OperationProgress, TextEncoding } from "../api/types";
import { useTransfer, type TransferRequest } from "../store/transfer";
import { Dialog } from "./Dialog";
import { formatCount } from "../lib/format";
import { assign, autoMap, toMapping, type MappingDraft } from "../lib/importMapping";

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
  const [includeCreate, setIncludeCreate] = useState(true);
  const [tableName, setTableName] = useState(req.source.kind === "table" ? req.source.table : "query_result");
  const op = useOperation<{ rows: number }>();

  const start = async () => {
    const ext = format === "csv" ? "csv" : format === "json" ? "json" : "sql";
    const path = await saveDialog({ defaultPath: `${req.label}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
    if (typeof path !== "string") return;
    const fmt: ExportFormat =
      format === "csv"
        ? { format: "csv", options: { delimiter: delimiter === "\\t" ? "\t" : delimiter, quote: '"', header, encoding, nullText } }
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
      {op.phase.state === "done" && <Outcome ok title={`Exported ${formatCount(op.phase.report.rows)} rows`} />}
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
        <Outcome ok={op.phase.report.failedCount === 0} title={`${formatCount(op.phase.report.inserted)} rows imported${op.phase.report.failedCount ? `, ${formatCount(op.phase.report.failedCount)} failed` : ""}`}>
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
    const p = await openDialog({ multiple: false, directory: false, filters: [{ name: "SQL", extensions: ["sql"] }] });
    if (typeof p === "string") setPath(p);
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
      {op.phase.state === "running" && <ProgressView done={op.phase.done} total={op.phase.total} unit="statements" />}
      {op.phase.state === "done" && (
        <Outcome ok={op.phase.report.failed.length === 0} title={`${formatCount(op.phase.report.executed)} statements executed${op.phase.report.failed.length ? `, ${op.phase.report.failed.length} failed` : ""}`}>
          {op.phase.report.failed.slice(0, 20).map((f) => <span key={f.index}>#{f.index + 1}: {f.message} — <code>{f.statement}</code></span>)}
        </Outcome>
      )}
      {op.phase.state === "failed" && <Outcome ok={false} title={op.phase.error.kind === "cancelled" ? "Stopped" : "Failed"}>{op.phase.error.kind !== "cancelled" && <span>{op.phase.error.message}</span>}</Outcome>}
    </Dialog>
  );
}
