import { useCallback, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import { MariaSQL, MySQL, SQLite, sql as sqlLang } from "@codemirror/lang-sql";
import { Prec } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { AlertCircle, CheckCircle2, FolderOpen, History, Play, Save, Square, TextSelect, Upload, X } from "lucide-react";
import { openTransfer } from "../store/transfer";
import { MOD } from "../lib/platform";
import { api, toAppError } from "../api/client";
import type { Engine, StatementOutcome } from "../api/types";
import { useTabs, type QueryTab } from "../store/tabs";
import { useQueryResults } from "../store/queryResults";
import { useConnection } from "../lib/useConnection";
import { notify, notifyError } from "../store/toasts";
import { GridView } from "./GridView";
import { formatCount } from "../lib/format";

export const ROW_LIMIT = 1000;
const SQL_FILTERS = [{ name: "SQL script", extensions: ["sql"] }];

const dialectFor = (e: Engine) => (e === "sqlite" ? SQLite : e === "mariaDb" ? MariaSQL : MySQL);
const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

export function QueryEditor({ tab }: { tab: QueryTab }) {
  const qc = useQueryClient();
  const conn = useConnection(tab.connectionId);
  const update = useTabs((s) => s.updateQuery);
  const run = useQueryResults((s) => s.runs[tab.id]);
  const { set: setRun, patch: patchRun } = useQueryResults();
  const editor = useRef<ReactCodeMirrorRef>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [resultsHeight, setResultsHeight] = useState(300);

  const databases = useQuery({ queryKey: ["databases", tab.connectionId], queryFn: () => api.listDatabases(tab.connectionId) });
  const columns = useQuery({
    queryKey: ["columns", tab.connectionId, tab.database],
    queryFn: () => api.listColumns(tab.connectionId, tab.database!),
    enabled: !!tab.database,
    staleTime: 60_000,
  });
  const schema = useMemo(() => Object.fromEntries(columns.data ?? []), [columns.data]);

  const execute = useCallback(
    async (onlySelection: boolean) => {
      const view = editor.current?.view;
      const state = view?.state;
      const selection = state ? state.sliceDoc(state.selection.main.from, state.selection.main.to) : "";
      const text = onlySelection && selection.trim() ? selection : (state?.doc.toString() ?? tab.sql);
      if (!text.trim() || useQueryResults.getState().runs[tab.id]?.running) return;
      const executionId = crypto.randomUUID();
      setRun(tab.id, { result: null, failure: null, running: true, executionId, startedAt: Date.now(), finishedAt: null });
      try {
        const result = await api.executeScript(tab.connectionId, tab.database, text, ROW_LIMIT, executionId);
        patchRun(tab.id, { result, running: false, finishedAt: Date.now() });
        // DDL and writes may have changed the schema or data shown elsewhere.
        void qc.invalidateQueries({ queryKey: ["objects", tab.connectionId] });
        void qc.invalidateQueries({ queryKey: ["databases", tab.connectionId] });
        void qc.invalidateQueries({ queryKey: ["columns", tab.connectionId] });
        void qc.invalidateQueries({ queryKey: ["history", tab.connectionId] });
      } catch (e) {
        patchRun(tab.id, { failure: toAppError(e), running: false, finishedAt: Date.now() });
      }
    },
    [tab.connectionId, tab.database, tab.id, tab.sql, qc, setRun, patchRun],
  );

  const stop = () => run?.executionId && void api.cancelExecution(tab.connectionId, run.executionId).catch((e) => notifyError("Could not cancel", e));

  const saveFile = async () => {
    try {
      const path = tab.filePath ?? (await saveDialog({ filters: SQL_FILTERS, defaultPath: `${tab.title}.sql` }));
      if (typeof path !== "string") return;
      await api.writeTextFile(path, tab.sql);
      update(tab.id, { filePath: path, title: fileName(path), savedSql: tab.sql });
      notify("Saved", fileName(path));
    } catch (e) {
      notifyError("Could not save the file", e);
    }
  };
  const openFile = async () => {
    const path = await openDialog({ filters: SQL_FILTERS, multiple: false, directory: false });
    if (typeof path !== "string") return;
    try {
      const sql = await api.readTextFile(path);
      useTabs.getState().newQuery({ connectionId: tab.connectionId, database: tab.database, sql, title: fileName(path), filePath: path });
    } catch (e) {
      notifyError("Could not open the file", e);
    }
  };

  const extensions = useMemo(
    () => [
      sqlLang({ dialect: dialectFor(conn?.engine ?? "mySql"), schema, upperCaseKeywords: true }),
      Prec.highest(
        keymap.of([
          { key: "Mod-Enter", run: () => (void execute(false), true) },
          { key: "Shift-Mod-Enter", run: () => (void execute(true), true) },
          { key: "Mod-s", run: () => (void saveFile(), true) },
        ]),
      ),
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [conn?.engine, schema, execute, tab.sql, tab.filePath],
  );

  return (
    <div className="query-editor">
      <div className="action-bar">
        <button className="btn primary" disabled={run?.running} onClick={() => void execute(false)} title={`Run (${MOD}+Enter)`}><Play size={15} />Run</button>
        <button className="btn" disabled={run?.running} onClick={() => void execute(true)} title={`Run selection (${MOD}+Shift+Enter)`}><TextSelect size={15} />Run Selection</button>
        <button className="btn danger" disabled={!run?.running} onClick={stop}><Square size={14} />Stop</button>
        <div className="toolbar-sep" />
        <label className="inline-field">
          <span>Database</span>
          <select className="input sm" value={tab.database ?? ""} aria-label="Database" onChange={(e) => update(tab.id, { database: e.target.value || null })}>
            {conn?.engine !== "sqlite" && <option value="">(none)</option>}
            {databases.data?.map((d) => <option key={d.name} value={d.name}>{d.name}</option>)}
          </select>
        </label>
        <div className="grow" />
        <button className="btn" onClick={openFile}><FolderOpen size={15} />Open</button>
        <button className="btn" onClick={saveFile} title={`Save (${MOD}+S)`}><Save size={15} />Save</button>
        <button className={`btn${showHistory ? " active-filter" : ""}`} onClick={() => setShowHistory((v) => !v)}><History size={15} />History</button>
      </div>
      <div className="query-main">
        <div className="query-left">
          <div className="query-code">
            <CodeMirror
              ref={editor}
              value={tab.sql}
              height="100%"
              extensions={extensions}
              onChange={(sql) => update(tab.id, { sql })}
              basicSetup={{ foldGutter: false, highlightActiveLine: true }}
              placeholder="SELECT * FROM …"
              autoFocus
            />
          </div>
          <HSplitter height={resultsHeight} onChange={setResultsHeight} />
          <div className="query-results" style={{ height: resultsHeight }}>
            <Results tabId={tab.id} connectionId={tab.connectionId} database={tab.database} />
          </div>
        </div>
        {showHistory && <HistoryPanel connectionId={tab.connectionId} onPick={(sql) => update(tab.id, { sql })} onClose={() => setShowHistory(false)} />}
      </div>
    </div>
  );
}

function HSplitter({ height, onChange }: { height: number; onChange: (h: number) => void }) {
  const start = useRef<{ y: number; h: number } | null>(null);
  return (
    <div
      className="hsplitter"
      role="separator"
      aria-orientation="horizontal"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { y: e.clientY, h: height };
      }}
      onPointerMove={(e) => start.current && onChange(Math.max(120, Math.min(900, start.current.h - (e.clientY - start.current.y))))}
      onPointerUp={() => (start.current = null)}
    />
  );
}

function Results({ tabId, connectionId, database }: { tabId: string; connectionId: string; database: string | null }) {
  const run = useQueryResults((s) => s.runs[tabId]);
  const patch = useQueryResults((s) => s.patch);
  const [selected, setSelected] = useState<number | "summary">("summary");
  const [loading, setLoading] = useState(false);
  const lastRun = useRef<number | null>(null);
  const outcomes = run?.result?.outcomes ?? [];
  const rowResults = outcomes.map((o, i) => ({ o, i })).filter((x) => x.o.kind === "rows");
  // Select the first result set (or the summary when the script failed) after each run.
  if (run?.finishedAt && lastRun.current !== run.finishedAt) {
    lastRun.current = run.finishedAt;
    const next = run.result?.error || run.failure || rowResults.length === 0 ? "summary" : rowResults[0].i;
    if (next !== selected) setSelected(next);
  }

  if (!run) return <div className="empty"><p>Run a query to see results here. {MOD}+Enter runs the script; {MOD}+Shift+Enter runs the selection.</p></div>;
  if (run.running) return <div className="empty"><p>Running… <span className="muted">({((Date.now() - run.startedAt) / 1000).toFixed(0)}s)</span></p></div>;

  const loadMore = async (index: number) => {
    const o = outcomes[index];
    if (o.kind !== "rows" || !run.result) return;
    setLoading(true);
    try {
      const page = await api.fetchMore(connectionId, database, o.statement, o.rows.length, ROW_LIMIT);
      const updated: StatementOutcome = { ...o, rows: [...o.rows, ...page.rows], hasMore: page.rows.length === ROW_LIMIT };
      patch(tabId, { result: { ...run.result, outcomes: outcomes.map((x, i) => (i === index ? updated : x)) } });
    } catch (e) {
      notifyError("Could not load more rows", e);
    } finally {
      setLoading(false);
    }
  };

  const current = typeof selected === "number" ? outcomes[selected] : undefined;
  return (
    <div className="results">
      <div className="result-tabs">
        <button className={selected === "summary" ? "on" : ""} onClick={() => setSelected("summary")}>
          {run.result?.error || run.failure ? <AlertCircle size={13} className="err" /> : <CheckCircle2 size={13} className="ok" />}Summary
        </button>
        {rowResults.map(({ i }, n) => (
          <button key={i} className={selected === i ? "on" : ""} onClick={() => setSelected(i)}>Result {n + 1}</button>
        ))}
      </div>
      {current?.kind === "rows" ? (
        <div className="result-grid">
          <GridView columns={current.columns} rows={current.rows} resetKey={current} emptyText="The query returned no rows." />
          <div className="result-footer">
            <span>{formatCount(current.rows.length)} row{current.rows.length === 1 ? "" : "s"}{current.hasMore ? " (more available)" : ""} · {current.durationMs} ms</span>
            {current.hasMore && <button className="btn sm" disabled={loading} onClick={() => void loadMore(selected as number)}>Load {ROW_LIMIT} more</button>}
            <div className="grow" />
            <button
              className="btn sm"
              disabled={!database}
              title={database ? "Export every row of this result" : "Choose a database to export"}
              onClick={() => database && openTransfer({ kind: "export", connectionId, database, source: { kind: "query", sql: current.statement }, label: "query_result" })}
            >
              <Upload size={13} />Export
            </button>
          </div>
        </div>
      ) : (
        <div className="summary">
          {run.failure && <div className="statement-row error"><AlertCircle size={15} /><div><b>{run.failure.message}</b></div></div>}
          {outcomes.map((o, i) => (
            <div key={i} className="statement-row">
              <CheckCircle2 size={15} className="ok" />
              <div>
                <code>{o.statement.length > 160 ? `${o.statement.slice(0, 160)}…` : o.statement}</code>
                <span className="muted">
                  {o.kind === "rows" ? `${formatCount(o.rows.length)}${o.hasMore ? "+" : ""} row(s) returned` : `${formatCount(o.rowsAffected)} row(s) affected`} · {o.durationMs} ms
                </span>
              </div>
            </div>
          ))}
          {run.result?.error && (
            <div className="statement-row error">
              <AlertCircle size={15} />
              <div>
                <code>{run.result.error.statement}</code>
                <b>{run.result.error.error.message}</b>
                <span className="muted">Statement {run.result.error.index + 1} failed; the remaining statements were not run.</span>
              </div>
            </div>
          )}
          {run.result && !run.result.error && outcomes.length === 0 && <p className="muted">No statements to run.</p>}
        </div>
      )}
    </div>
  );
}

function HistoryPanel({ connectionId, onPick, onClose }: { connectionId: string; onPick: (sql: string) => void; onClose: () => void }) {
  const history = useQuery({ queryKey: ["history", connectionId], queryFn: () => api.listHistory(connectionId) });
  return (
    <aside className="history-panel">
      <div className="panel-header"><span className="grow">History</span><button className="icon-btn" aria-label="Close history" onClick={onClose}><X size={14} /></button></div>
      <div className="history-list">
        {history.data?.map((h, i) => (
          <button key={i} className="history-item" title={h.sql} onClick={() => onPick(h.sql)}>
            <span className="history-meta">
              {h.ok ? <CheckCircle2 size={12} className="ok" /> : <AlertCircle size={12} className="err" />}
              {new Date(h.at).toLocaleString()} · {h.durationMs} ms
            </span>
            <code>{h.sql}</code>
          </button>
        ))}
        {history.data?.length === 0 && <p className="muted pad">No queries yet.</p>}
      </div>
    </aside>
  );
}
