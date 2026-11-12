import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type FocusEvent, type Ref } from "react";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Upload, Ban, Check, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Filter as FilterIcon, Lock, Plus, RefreshCw, Sparkles, Trash2, Undo2, X } from "lucide-react";
import { api } from "../api/client";
import type { CellValue, ChangeError, Filter, FilterOp, Page, SortSpec, TableInfo } from "../api/types";
import { useTabs, type TableTab } from "../store/tabs";
import { formatCount } from "../lib/format";
import { notify, notifyError } from "../store/toasts";
import { confirmAction } from "../store/confirm";
import { commitPending, useGridState, useGridStates, type DraftFilter } from "../store/gridState";
import { openTransfer } from "../store/transfer";
import { MOD, isMod } from "../lib/platform";
import { useRefreshShortcut } from "../lib/refreshShortcut";
import { useRefetchFlash } from "../lib/refetchFlash";
import { GridView, ROW_HEIGHT, type GridSelection } from "./GridView";
import { useTableInfo } from "./TableTabView";
import { DatePicker, dateKind, type DateKind } from "./DatePicker";
import { useConnection } from "../lib/useConnection";
import { editedCell, editorKeyAction, editorText } from "../lib/cells";
import {
  addRow,
  isUnset,
  buildChangeSet,
  cellValue,
  changeCount,
  deleteRows,
  emptyPending,
  isDeleted,
  isDirty,
  isEdited,
  setCell,
  type Pending,
  type RowRef,
} from "../lib/pendingChanges";

/** Page size meaning "every row": a LIMIT both MySQL and SQLite accept, and that keeps the pager on one page. */
const ALL_ROWS = Number.MAX_SAFE_INTEGER;
const PAGE_SIZES = [
  { value: 100, label: "100 / page" },
  { value: 500, label: "500 / page" },
  { value: 1000, label: "1000 / page" },
  { value: ALL_ROWS, label: "All rows" },
];

const OPS: { op: FilterOp; label: string; needsValue: boolean }[] = [
  { op: "eq", label: "=", needsValue: true },
  { op: "notEq", label: "≠", needsValue: true },
  { op: "lt", label: "<", needsValue: true },
  { op: "lte", label: "≤", needsValue: true },
  { op: "gt", label: ">", needsValue: true },
  { op: "gte", label: "≥", needsValue: true },
  { op: "contains", label: "contains", needsValue: true },
  { op: "startsWith", label: "starts with", needsValue: true },
  { op: "isNull", label: "is NULL", needsValue: false },
  { op: "isNotNull", label: "is not NULL", needsValue: false },
];

export function DataGrid({ tab }: { tab: TableTab }) {
  const qc = useQueryClient();
  const setRowCount = useTabs((s) => s.setRowCount);
  const [grid, update] = useGridState(tab.id);
  const { pageSize, offset, sort, filters, drafts, showFilters, pending, failedRow, snapshot, snapshotInfo, committing } = grid;
  const setOffset = (offset: number) => update(() => ({ offset }));
  const setPageSize = (pageSize: number) => update(() => ({ pageSize }));
  const setSort = (f: (s: SortSpec | null) => SortSpec | null) => update((g) => ({ sort: f(g.sort) }));
  const setFilters = (filters: Filter[]) => update(() => ({ filters }));
  const setDrafts = (f: DraftFilter[] | ((d: DraftFilter[]) => DraftFilter[])) => update((g) => ({ drafts: typeof f === "function" ? f(g.drafts) : f }));
  const setShowFilters = (f: (v: boolean) => boolean) => update((g) => ({ showFilters: f(g.showFilters) }));
  const setPending = (f: Pending | ((p: Pending) => Pending)) =>
    update((g) => {
      const next = typeof f === "function" ? f(g.pending) : f;
      // Freeze the page the edits refer to (the one an open editor froze); release it once nothing is pending.
      return {
        pending: next,
        snapshot: isDirty(next) ? (g.snapshot ?? data ?? null) : null,
        snapshotInfo: isDirty(next) ? (g.snapshotInfo ?? tableInfo ?? null) : null,
      };
    });
  const setFailedRow = (failedRow: RowRef | null) => update(() => ({ failedRow }));
  /**
   * The open editor keeps the page and table info it opened on, so a background reload cannot move its row. `targets`
   * are the cells its text is saved to: the edited cell, plus the rest of the selection when Enter opened it there.
   */
  const [editing, setEditing] = useState<{ row: number; column: string; targets: CellRef[]; session: number; page: Page | undefined; info: TableInfo | undefined } | null>(null);
  const [selection, setSelection] = useState<GridSelection>({ rows: [], columns: [] });
  const editorRef = useRef<CellEditorHandle | null>(null);
  const editSession = useRef(0);
  const isCommitting = () => !!useGridStates.getState().states[tab.id]?.committing;

  const engine = useConnection(tab.connectionId)?.engine;
  const info = useTableInfo(tab);
  const tableInfo = snapshotInfo ?? editing?.info ?? info.data;
  const identity = tableInfo?.rowIdentity;
  const readOnly = !identity || identity.kind === "none";

  const req = { offset, limit: pageSize, sort, filters };
  const base = [tab.connectionId, tab.database, tab.name] as const;
  const page = useQuery({
    queryKey: ["page", ...base, req],
    queryFn: () => api.fetchPage(tab.connectionId, tab.database, tab.name, req),
    placeholderData: keepPreviousData,
  });
  const gridRef = useRef<HTMLDivElement | null>(null);
  // Placeholder rows from the previous page/sort/filter must not count as a refetch of the new one.
  useRefetchFlash(gridRef, JSON.stringify(["page", ...base, req]), page.isPlaceholderData ? 0 : page.dataUpdatedAt, page.isSuccess);
  const count = useQuery({
    queryKey: ["count", ...base, filters],
    queryFn: () => api.countRows(tab.connectionId, tab.database, tab.name, filters),
  });
  useEffect(() => setRowCount(tab.id, count.data), [count.data, tab.id, setRowCount]);

  const data = snapshot ?? editing?.page ?? page.data;
  const columns = useMemo(() => data?.columns ?? [], [data]);
  const pageRows = useMemo(() => data?.rows ?? [], [data]);
  const saved = pageRows.length;
  const rows = useMemo(
    () => [...pageRows, ...pending.inserts.map(() => columns.map((): CellValue => ({ type: "null" })))],
    [pageRows, pending.inserts, columns],
  );
  const refOf = (r: number): RowRef => (r < saved ? { kind: "existing", index: r } : { kind: "new", index: r - saved });
  const colIndex = (name: string) => columns.findIndex((c) => c.name === name);
  const original = (r: number, column: string): CellValue => (r < saved ? pageRows[r][colIndex(column)] : { type: "null" });

  const dirty = isDirty(pending);
  const guard = async () => {
    if (isCommitting()) return false;
    editorRef.current?.finish();
    const current = useGridStates.getState().states[tab.id]?.pending ?? emptyPending();
    if (!isDirty(current)) return true;
    const allowed = await confirmAction({ title: "Discard changes", message: `Discard ${changeCount(current)} pending change${changeCount(current) === 1 ? "" : "s"}?`, confirmLabel: "Discard", danger: true });
    return allowed && !isCommitting();
  };
  const reset = () => {
    if (!useGridStates.getState().discard(tab.id)) return false;
    setEditing(null);
    return true;
  };
  const go = async (o: number) => {
    if (!(await guard())) return;
    if (!reset()) return;
    setOffset(Math.max(0, o));
  };
  const refresh = async () => {
    if (!(await guard())) return;
    if (!reset()) return;
    void qc.invalidateQueries({ queryKey: ["page", ...base] });
    void qc.invalidateQueries({ queryKey: ["count", ...base] });
  };
  useRefreshShortcut(() => void refresh());
  const toggleSort = async (column: string) => {
    if (!(await guard())) return;
    if (!reset()) return;
    setSort((s) => (s?.column !== column ? { column, desc: false } : !s.desc ? { column, desc: true } : null));
    setOffset(0);
  };
  const applyFilters = async () => {
    if (!(await guard())) return;
    if (!reset()) return;
    setFilters(
      drafts
        .filter((d) => d.column)
        .map((d) => ({ column: d.column, op: d.op, value: OPS.find((o) => o.op === d.op)!.needsValue ? { type: "text", value: d.value } : null })),
    );
    setOffset(0);
  };

  const total = count.data;
  const lastOffset = total !== undefined ? Math.max(0, Math.floor((total - 1) / pageSize) * pageSize) : offset;

  /**
   * Opens the cell editor; `quiet` (a plain click) skips the explanation when the cell cannot be edited. `spread`
   * (Enter, double-click) saves the text to every selected cell when the cell is part of a larger selection.
   */
  const startEdit = useCallback(
    (row: number, column: string, quiet = false, spread = false) => {
      if (isCommitting() || page.isPlaceholderData) return;
      if (readOnly) {
        if (!quiet) notifyError("This data is read-only", { message: tableInfo?.kind === "view" ? "Views are not editable." : "The table has no key and no column whose value can identify a row." });
        return;
      }
      if (tableInfo?.columns.find((c) => c.name === column)?.generated) {
        if (!quiet) notifyError("Generated columns are read-only", { message: "The database computes this value." });
        return;
      }
      const v = original(row, column);
      if (v.type === "bytes" && row < saved) {
        if (!quiet) notifyError("Binary values cannot be edited here", { message: "Set the cell to NULL or use the SQL editor." });
        return;
      }
      // Binary, generated and deleted cells in the selection keep their values.
      const targets: CellRef[] = [{ row, column }];
      if (spread && selection.rows.includes(row) && selection.columns.includes(column)) {
        for (const r of selection.rows) {
          for (const c of selection.columns) {
            if (r === row && c === column) continue;
            if (tableInfo?.columns.find((col) => col.name === c)?.generated) continue;
            if (r < saved && (original(r, c).type === "bytes" || isDeleted(pending, refOf(r)))) continue;
            targets.push({ row: r, column: c });
          }
        }
      }
      setEditing({ row, column, targets, session: ++editSession.current, page: data, info: tableInfo });
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab.id, readOnly, data, tableInfo, page.isPlaceholderData, selection, pending],
  );
  const write = (row: number, column: string, value: CellValue) => {
    if (readOnly || isCommitting() || page.isPlaceholderData) return;
    if (tableInfo?.columns.find((c) => c.name === column)?.generated) return;
    setPending((p) => setCell(p, refOf(row), column, value, original(row, column)));
    setFailedRow(null);
  };
  /** Adds a row prefilled with the column defaults, optionally editing `column` right away. */
  const appendRow = (column?: string) => {
    if (readOnly || !engine || isCommitting() || page.isPlaceholderData) return;
    const r = addRow(useGridStates.getState().states[tab.id]?.pending ?? emptyPending(), tableInfo?.columns ?? [], engine);
    setPending(r.pending);
    if (column) setEditing({ row: saved + r.row.index, column, targets: [{ row: saved + r.row.index, column }], session: ++editSession.current, page: data, info: tableInfo });
  };
  // ArrowDown past the last row adds one, unless that row is itself a new row nobody has filled in yet.
  const lastInsert = pending.inserts.at(-1);
  const lastRowUntouched = lastInsert !== undefined && !!engine && JSON.stringify(lastInsert) === JSON.stringify(addRow(emptyPending(), tableInfo?.columns ?? [], engine).pending.inserts[0]);

  const commit = async () => {
    if (!identity || identity.kind === "none" || !data || isCommitting() || page.isPlaceholderData) return;
    // Ending the editor updates Zustand synchronously; read the change set only afterwards.
    editorRef.current?.finish();
    await commitPending(tab.id, async (submitted) => {
      // Set once the change set is built: a failure before that is not about any row.
      let rowOf: RowRef[] | null = null;
      let saved = false;
      try {
        const submittedData = submitted.snapshot ?? data;
        const submittedIdentity = submitted.snapshotInfo?.rowIdentity ?? identity;
        const identityColumns = submittedIdentity.kind === "rowId" ? ["__rowid__"] : submittedIdentity.kind === "none" ? [] : submittedIdentity.columns;
        const built = buildChangeSet(submitted.pending, submittedData.rows, submittedData.columns, identityColumns);
        rowOf = built.rowOf;
        await api.applyChanges(tab.connectionId, tab.database, tab.name, submittedIdentity, built.changes);
        saved = true;
        notify("Changes saved", `${built.changes.length} change${built.changes.length === 1 ? "" : "s"} applied`);
        await qc.invalidateQueries({ queryKey: ["page", ...base] });
        await qc.invalidateQueries({ queryKey: ["count", ...base] });
        void qc.invalidateQueries({ queryKey: ["objects", tab.connectionId, tab.database] });
        return { saved, failedRow: null };
      } catch (e) {
        if (saved) {
          notifyError("Changes saved, but the grid could not be refreshed", e);
          return { saved, failedRow: null };
        }
        if (!rowOf) {
          notifyError("Changes were not saved", e);
          return { saved, failedRow: null };
        }
        const err = e as ChangeError;
        notifyError("Changes were not saved", err.error);
        return { saved, failedRow: err.index !== null ? rowOf[err.index] ?? null : null };
      }
    });
  };
  const removeRows = (selected: readonly number[]) => {
    if (readOnly || isCommitting() || page.isPlaceholderData || selected.length === 0) return;
    const refs = selected.filter((row) => row >= 0 && row < rows.length).map(refOf);
    setPending((current) => deleteRows(current, refs));
    setEditing(null);
  };

  const sameRef = (a: RowRef | null, b: RowRef) => !!a && a.kind === b.kind && a.index === b.index;

  return (
    <div
      className="grid-wrap"
      onKeyDown={(e) => {
        if (isMod(e) && e.key.toLowerCase() === "s") {
          e.preventDefault();
          void commit();
          if (editing) requestAnimationFrame(() => gridRef.current?.focus({ preventScroll: true }));
        }
      }}
    >
      <div className="action-bar grid-toolbar">
        {readOnly ? (
          info.data && <span className="badge"><Lock size={13} />Read-only — {info.data.kind === "view" ? "views are not editable" : "no key or matchable column"}</span>
        ) : (
          <>
            <button className="obj-btn" disabled={committing || page.isPlaceholderData} onClick={() => appendRow(columns.find((c) => !c.hidden)?.name ?? "")}>
              <Plus size={15} className="ob-new" />Add Row
            </button>
            <button className="obj-btn" disabled={selection.rows.length === 0 || committing || page.isPlaceholderData} onClick={() => removeRows(selection.rows)}>
              <Trash2 size={15} className="ob-delete" />Delete
            </button>
            <button className="obj-btn" disabled={selection.rows.length === 0 || committing || page.isPlaceholderData || selection.columns.every((name) => tableInfo?.columns.find((c) => c.name === name)?.generated)} title="Set selected cells to NULL" onClick={() => selection.rows.forEach((r) => selection.columns.forEach((c) => write(r, c, { type: "null" })))}>
              <Ban size={15} />Set NULL
            </button>
            <div className="toolbar-sep" />
            <button className="obj-btn commit" disabled={(!dirty && !editing) || committing || page.isPlaceholderData} onClick={commit} title={`Commit (${MOD}+S)`}>
              <Check size={15} className="ob-new" />Commit{dirty ? ` (${changeCount(pending)})` : ""}
            </button>
            <button className="obj-btn" disabled={!dirty || committing} onClick={reset}><Undo2 size={15} className="ob-delete" />Discard</button>
          </>
        )}
        <button className="obj-btn" title="Export this table" onClick={() => openTransfer({ kind: "export", connectionId: tab.connectionId, database: tab.database, source: { kind: "table", table: tab.name }, label: tab.name })}>
          <Upload size={15} className="ob-design" />Export
        </button>
        {!readOnly && (
          <button className="obj-btn" disabled={committing} title="Import a CSV file" onClick={() => openTransfer({ kind: "import", connectionId: tab.connectionId, database: tab.database, table: tab.name })}>
            <Download size={15} className="ob-new" />Import
          </button>
        )}
        {tab.objectKind === "table" && (
          <button className="obj-btn" disabled={committing} title="Fill the table with realistic test data" onClick={() => openTransfer({ kind: "generate", connectionId: tab.connectionId, database: tab.database, table: tab.name })}>
            <Sparkles size={15} className="ob-design" />Generate
          </button>
        )}
        <button className={`obj-btn${showFilters || filters.length ? " active-filter" : ""}`} disabled={committing} onClick={() => setShowFilters((v) => !v)}>
          <FilterIcon size={15} className="ob-open" />Filter{filters.length ? ` (${filters.length})` : ""}
        </button>
      </div>
      {showFilters && (
        <div className="filter-bar">
          {drafts.map((d, i) => {
            const needsValue = OPS.find((o) => o.op === d.op)!.needsValue;
            const patch = (p: Partial<DraftFilter>) => setDrafts((ds) => ds.map((x, j) => (j === i ? { ...x, ...p } : x)));
            return (
              <div className="filter-row" key={i}>
                <span className="muted filter-join">{i === 0 ? "Where" : "and"}</span>
                <select className="input sm" disabled={committing} value={d.column} aria-label="Filter column" onChange={(e) => patch({ column: e.target.value })}>
                  {columns.filter((c) => !c.hidden).map((c) => <option key={c.name} value={c.name}>{c.name}</option>)}
                </select>
                <select className="input sm" disabled={committing} value={d.op} aria-label="Filter operator" onChange={(e) => patch({ op: e.target.value as FilterOp })}>
                  {OPS.map((o) => <option key={o.op} value={o.op}>{o.label}</option>)}
                </select>
                {needsValue && (
                  <input className="input sm" disabled={committing} value={d.value} aria-label="Filter value" placeholder="value" onChange={(e) => patch({ value: e.target.value })} onKeyDown={(e) => e.key === "Enter" && void applyFilters()} />
                )}
                <button className="icon-btn" disabled={committing} aria-label="Remove filter" onClick={() => setDrafts((ds) => ds.filter((_, j) => j !== i))}><X size={14} /></button>
              </div>
            );
          })}
          <div className="filter-actions">
            <button className="btn sm" disabled={committing} onClick={() => setDrafts((ds) => [...ds, { column: columns.find((c) => !c.hidden)?.name ?? "", op: "eq", value: "" }])}><Plus size={13} />Add condition</button>
            <button className="btn sm primary" disabled={committing} onClick={applyFilters}>Apply</button>
            {filters.length > 0 && <button className="btn sm ghost" disabled={committing} onClick={async () => { if (!(await guard()) || !reset()) return; setDrafts([]); setFilters([]); setOffset(0); }}>Clear</button>}
          </div>
        </div>
      )}
      {page.isError ? (
        <div className="empty"><p className="error-text">{(page.error as { message: string }).message}</p></div>
      ) : (
        <GridView
          ref={gridRef}
          columns={columns}
          rows={rows}
          savedRowCount={saved}
          rowNumberOffset={offset}
          sort={sort}
          onSort={committing ? undefined : toggleSort}
          resetKey={data}
          onSelectionChange={setSelection}
          onDeleteRows={readOnly || committing || page.isPlaceholderData ? undefined : removeRows}
          emptyText={page.isSuccess ? `No rows${filters.length ? " match the filter" : ""}.` : undefined}
          cellValue={(r, column, orig) => cellValue(pending, refOf(r), column, r < saved ? orig : undefined)}
          rowClass={(r) => {
            const ref = refOf(r);
            if (sameRef(failedRow, ref)) return "row-failed";
            if (ref.kind === "new") return "row-new";
            return isDeleted(pending, ref) ? "row-deleted" : undefined;
          }}
          cellClass={(r, column) => (isEdited(pending, refOf(r), column) ? "cell-edited" : undefined)}
          cellPlaceholder={(r, column) => (isUnset(pending, refOf(r), column) ? "(Default)" : undefined)}
          onCellActivate={committing ? undefined : (r, column) => startEdit(r, column, false, true)}
          onCellEdit={committing ? undefined : (r, column) => startEdit(r, column, true)}
          onAppendRow={readOnly || committing || page.isPlaceholderData || lastRowUntouched ? undefined : appendRow}
          renderEditor={(r, column) =>
            editing?.row === r && editing.column === column ? (
              <CellEditor
                key={editing.session}
                ref={editorRef}
                disabled={committing || !!tableInfo?.columns.find((c) => c.name === column)?.generated}
                kind={dateKind(columns[colIndex(column)]?.typeName ?? "")}
                initial={cellValue(pending, refOf(r), column, r < saved ? pageRows[r][colIndex(column)] : undefined)}
                onDone={(typed) => {
                  // Each cell compares the text with its own value, so cells that already hold it stay unchanged.
                  for (const t of editing.targets) {
                    const v = editedCell(editorText(cellValue(pending, refOf(t.row), t.column, original(t.row, t.column))), typed);
                    if (v) write(t.row, t.column, v);
                  }
                  setEditing(null);
                }}
              />
            ) : null
          }
        />
      )}
      <div className="grid-footer">
        <span className="muted" title={identity?.kind === "allColumns" ? "This table has no primary or unique key: edits find each row by its column values and change only one row." : undefined}>
          {readOnly ? "Read-only" : dirty ? `${changeCount(pending)} pending` : identity?.kind === "allColumns" ? "No key: rows are matched by value" : ""}
        </span>
        <div className="grow" />
        <div className="pager">
          <button className="icon-btn" title="First page" aria-label="First page" disabled={committing || offset === 0} onClick={() => go(0)}><ChevronsLeft size={16} /></button>
          <button className="icon-btn" title="Previous page" aria-label="Previous page" disabled={committing || offset === 0} onClick={() => go(offset - pageSize)}><ChevronLeft size={16} /></button>
          <span className="pager-text">{formatCount(saved ? offset + 1 : 0)}–{formatCount(offset + saved)} of {total === undefined ? "…" : formatCount(total)}</span>
          <button className="icon-btn" title="Next page" aria-label="Next page" disabled={committing || total === undefined || offset + pageSize >= total} onClick={() => go(offset + pageSize)}><ChevronRight size={16} /></button>
          <button className="icon-btn" title="Last page" aria-label="Last page" disabled={committing || total === undefined || offset >= lastOffset} onClick={() => go(lastOffset)}><ChevronsRight size={16} /></button>
          <select className="input sm" disabled={committing} value={pageSize} aria-label="Rows per page" onChange={async (e) => { const n = Number(e.target.value); if (!(await guard()) || !reset()) return; setPageSize(n); setOffset(0); }}>
            {PAGE_SIZES.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        </div>
        <button className="icon-btn" disabled={committing} title="Refresh" aria-label="Refresh" onClick={refresh}><RefreshCw size={15} className={page.isFetching ? "spin" : undefined} /></button>
      </div>
    </div>
  );
}

type CellEditorHandle = { finish: () => void };
type CellRef = { row: number; column: string };

/** Lines a multi-line value shows at once; longer text scrolls inside the editor. */
const EDITOR_MAX_LINES = 8;
const EDITOR_LINE_HEIGHT = 18;

/**
 * In-cell text editor. A textarea, so line breaks survive (an `<input>` strips them): Enter saves, Shift+Enter or
 * Alt+Enter adds a line, and a value with several lines opens over the rows below. `onDone` gets the typed text, or
 * null when nothing was typed or the edit was cancelled.
 */
function CellEditor({ initial, kind, disabled, onDone, ref }: {
  initial: CellValue;
  kind: DateKind | null;
  disabled: boolean;
  onDone: (typed: string | null) => void;
  ref?: Ref<CellEditorHandle>;
}) {
  const start = editorText(initial);
  const [text, setText] = useState(start);
  // What the user typed, null until the first change. Saving reads this, never the control's value, so nothing
  // the control normalizes counts as an edit.
  const typed = useRef<string | null>(null);
  const [pickerAt, setPickerAt] = useState<DOMRect | null>(null);
  const [width, setWidth] = useState<number | undefined>(undefined);
  const wrap = useRef<HTMLDivElement>(null);
  const picker = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const done = useRef(false);
  useLayoutEffect(() => setWidth(wrap.current?.offsetWidth), []);
  const finish = (value: string | null) => {
    if (disabled || done.current) return;
    done.current = true;
    onDone(value);
  };
  const change = (value: string) => {
    if (disabled || done.current) return;
    typed.current = value;
    setText(value);
  };
  useImperativeHandle(ref, () => ({ finish: () => finish(typed.current) }));
  /** Focus leaving both the editor and its picker ends the edit, saving it. */
  const onBlur = (e: FocusEvent) => {
    const to = e.relatedTarget as Node | null;
    if (to && (wrap.current?.contains(to) || picker.current?.contains(to))) return;
    finish(typed.current);
  };
  // Keys go to the picker while it is open (Esc closes it, Enter picks); the editor gets them back on Cancel.
  const openPicker = () => {
    if (disabled || done.current) return;
    setPickerAt(wrap.current?.getBoundingClientRect() ?? null);
    requestAnimationFrame(() => picker.current?.focus());
  };
  const lines = text.split(/\r\n?|\n/).length;
  return (
    <div className="cell-editor-wrap" ref={wrap}>
      <textarea
        ref={input}
        className="cell-editor"
        autoFocus
        rows={1}
        disabled={disabled}
        value={text}
        placeholder={initial.type === "null" ? "NULL" : undefined}
        style={{
          display: "block",
          resize: "none",
          whiteSpace: "pre",
          ...(lines > 1
            ? {
                position: "absolute",
                zIndex: 1,
                width: width === undefined ? undefined : width - (kind ? 24 : 0),
                height: Math.min(lines, EDITOR_MAX_LINES) * EDITOR_LINE_HEIGHT + 8,
                padding: "4px 8px",
                lineHeight: `${EDITOR_LINE_HEIGHT}px`,
                overflow: "auto",
                background: "var(--surface)",
                boxShadow: "0 0 0 1px var(--accent), 0 4px 12px rgb(0 0 0 / 0.25)",
              }
            : { lineHeight: `${ROW_HEIGHT - 1}px`, overflow: "hidden" }),
        }}
        onChange={(e) => change(e.target.value)}
        onBlur={onBlur}
        onKeyDown={(e) => {
          if (disabled || done.current) return;
          const action = editorKeyAction({ key: e.key, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey, metaKey: e.metaKey, isComposing: e.nativeEvent.isComposing }, !!kind);
          switch (action) {
            case "picker":
              e.preventDefault();
              openPicker();
              break;
            case "newline": {
              // Shift+Enter inserts the line break natively; Alt+Enter does not.
              if (!e.altKey) break;
              e.preventDefault();
              const el = e.currentTarget;
              el.setRangeText("\n", el.selectionStart, el.selectionEnd, "end");
              change(el.value);
              break;
            }
            case "commit":
              // Tab also saves; the grid then moves the editor to the neighbouring cell.
              if (e.key === "Enter") e.preventDefault();
              finish(typed.current);
              break;
            case "cancel":
              finish(null);
              break;
            case null:
              break;
            default: {
              const never: never = action;
              return never;
            }
          }
        }}
      />
      {kind && (
        <button
          className="cell-editor-btn"
          disabled={disabled}
          tabIndex={-1}
          title="Pick a date (Alt+↓)"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => (pickerAt ? setPickerAt(null) : openPicker())}
        >
          …
        </button>
      )}
      {kind && pickerAt && !disabled && (
        <DatePicker
          ref={picker}
          kind={kind}
          value={text}
          anchor={pickerAt}
          onBlur={onBlur}
          onPick={(value) => {
            const grid = wrap.current?.closest<HTMLElement>(".grid");
            finish(value);
            // Accept the picked value before focus triggers blur with the previous input text.
            grid?.focus({ preventScroll: true });
          }}
          onCancel={() => {
            setPickerAt(null);
            input.current?.focus();
          }}
        />
      )}
    </div>
  );
}
