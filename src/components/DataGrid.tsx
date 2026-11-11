import { useCallback, useEffect, useMemo, useRef, useState, type FocusEvent } from "react";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Upload, Ban, Check, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Filter as FilterIcon, Lock, Plus, RefreshCw, Sparkles, Trash2, Undo2, X } from "lucide-react";
import { api } from "../api/client";
import type { CellValue, ChangeError, Filter, FilterOp, SortSpec } from "../api/types";
import { useTabs, type TableTab } from "../store/tabs";
import { formatCount } from "../lib/format";
import { notify, notifyError } from "../store/toasts";
import { confirmAction } from "../store/confirm";
import { useGridState, type DraftFilter } from "../store/gridState";
import { openTransfer } from "../store/transfer";
import { MOD, isMod } from "../lib/platform";
import { useRefreshShortcut } from "../lib/refreshShortcut";
import { useRefetchFlash } from "../lib/refetchFlash";
import { GridView, type GridSelection } from "./GridView";
import { useTableInfo } from "./TableTabView";
import { DatePicker, dateKind, type DateKind } from "./DatePicker";
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
  const { pageSize, offset, sort, filters, drafts, showFilters, pending, failedRow, snapshot } = grid;
  const setOffset = (offset: number) => update(() => ({ offset }));
  const setPageSize = (pageSize: number) => update(() => ({ pageSize }));
  const setSort = (f: (s: SortSpec | null) => SortSpec | null) => update((g) => ({ sort: f(g.sort) }));
  const setFilters = (filters: Filter[]) => update(() => ({ filters }));
  const setDrafts = (f: DraftFilter[] | ((d: DraftFilter[]) => DraftFilter[])) => update((g) => ({ drafts: typeof f === "function" ? f(g.drafts) : f }));
  const setShowFilters = (f: (v: boolean) => boolean) => update((g) => ({ showFilters: f(g.showFilters) }));
  const setPending = (f: Pending | ((p: Pending) => Pending)) =>
    update((g) => {
      const next = typeof f === "function" ? f(g.pending) : f;
      // Freeze the page the edits refer to; release it once nothing is pending.
      return { pending: next, snapshot: isDirty(next) ? (g.snapshot ?? page.data ?? null) : null };
    });
  const setFailedRow = (failedRow: RowRef | null) => update(() => ({ failedRow }));
  const [editing, setEditing] = useState<{ row: number; column: string } | null>(null);
  const [selection, setSelection] = useState<GridSelection>({ rows: [], columns: [] });
  const [committing, setCommitting] = useState(false);

  const info = useTableInfo(tab);
  const identity = info.data?.rowIdentity;
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

  const data = snapshot ?? page.data;
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
  const guard = async () =>
    !dirty ||
    (await confirmAction({ title: "Discard changes", message: `Discard ${changeCount(pending)} pending change${changeCount(pending) === 1 ? "" : "s"}?`, confirmLabel: "Discard", danger: true }));
  const reset = () => {
    setPending(emptyPending());
    setEditing(null);
    setFailedRow(null);
  };
  const go = async (o: number) => {
    if (!(await guard())) return;
    reset();
    setOffset(Math.max(0, o));
  };
  const refresh = async () => {
    if (!(await guard())) return;
    reset();
    void qc.invalidateQueries({ queryKey: ["page", ...base] });
    void qc.invalidateQueries({ queryKey: ["count", ...base] });
  };
  useRefreshShortcut(() => void refresh());
  const toggleSort = async (column: string) => {
    if (!(await guard())) return;
    reset();
    setSort((s) => (s?.column !== column ? { column, desc: false } : !s.desc ? { column, desc: true } : null));
    setOffset(0);
  };
  const applyFilters = async () => {
    if (!(await guard())) return;
    reset();
    setFilters(
      drafts
        .filter((d) => d.column)
        .map((d) => ({ column: d.column, op: d.op, value: OPS.find((o) => o.op === d.op)!.needsValue ? { type: "text", value: d.value } : null })),
    );
    setOffset(0);
  };

  const total = count.data;
  const lastOffset = total !== undefined ? Math.max(0, Math.floor((total - 1) / pageSize) * pageSize) : offset;

  /** Opens the cell editor; `quiet` (a plain click) skips the explanation when the cell cannot be edited. */
  const startEdit = useCallback(
    (row: number, column: string, quiet = false) => {
      if (readOnly) {
        if (!quiet) notifyError("This data is read-only", { message: info.data?.kind === "view" ? "Views are not editable." : "The table has no key and no column whose value can identify a row." });
        return;
      }
      const v = original(row, column);
      if (v.type === "bytes" && row < saved) {
        if (!quiet) notifyError("Binary values cannot be edited here", { message: "Set the cell to NULL or use the SQL editor." });
        return;
      }
      setEditing({ row, column });
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [readOnly, pageRows, columns],
  );
  const write = (row: number, column: string, value: CellValue) => {
    setPending((p) => setCell(p, refOf(row), column, value, original(row, column)));
    setFailedRow(null);
  };
  /** Adds a row prefilled with the column defaults, optionally editing `column` right away. */
  const appendRow = (column?: string) => {
    const r = addRow(pending, info.data?.columns ?? []);
    setPending(r.pending);
    if (column) setEditing({ row: saved + r.row.index, column });
  };
  // ArrowDown past the last row adds one, unless that row is itself a new row nobody has filled in yet.
  const lastInsert = pending.inserts.at(-1);
  const lastRowUntouched = lastInsert !== undefined && JSON.stringify(lastInsert) === JSON.stringify(addRow(emptyPending(), info.data?.columns ?? []).pending.inserts[0]);

  const commit = async () => {
    if (!identity || !data) return;
    const identityColumns = identity.kind === "rowId" ? ["__rowid__"] : identity.kind === "none" ? [] : identity.columns;
    const { changes, rowOf } = buildChangeSet(pending, pageRows, columns, identityColumns);
    setCommitting(true);
    try {
      await api.applyChanges(tab.connectionId, tab.database, tab.name, changes);
      notify("Changes saved", `${changes.length} change${changes.length === 1 ? "" : "s"} applied`);
      reset();
      await qc.invalidateQueries({ queryKey: ["page", ...base] });
      await qc.invalidateQueries({ queryKey: ["count", ...base] });
      void qc.invalidateQueries({ queryKey: ["objects", tab.connectionId, tab.database] });
    } catch (e) {
      const err = e as ChangeError;
      setFailedRow(err.index !== null ? rowOf[err.index] : null);
      notifyError("Changes were not saved", err.error);
    } finally {
      setCommitting(false);
    }
  };
  const removeRows = (selected: readonly number[]) => {
    if (readOnly || committing || page.isPlaceholderData || selected.length === 0) return;
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
          if (dirty && !committing) void commit();
        }
      }}
    >
      <div className="action-bar grid-toolbar">
        {readOnly ? (
          info.data && <span className="badge"><Lock size={13} />Read-only — {info.data.kind === "view" ? "views are not editable" : "no key or matchable column"}</span>
        ) : (
          <>
            <button className="obj-btn" onClick={() => appendRow(columns.find((c) => !c.hidden)?.name ?? "")}>
              <Plus size={15} className="ob-new" />Add Row
            </button>
            <button className="obj-btn" disabled={selection.rows.length === 0 || committing || page.isPlaceholderData} onClick={() => removeRows(selection.rows)}>
              <Trash2 size={15} className="ob-delete" />Delete
            </button>
            <button className="obj-btn" disabled={selection.rows.length === 0} title="Set selected cells to NULL" onClick={() => selection.rows.forEach((r) => selection.columns.forEach((c) => write(r, c, { type: "null" })))}>
              <Ban size={15} />Set NULL
            </button>
            <div className="toolbar-sep" />
            <button className="obj-btn commit" disabled={!dirty || committing} onClick={commit} title={`Commit (${MOD}+S)`}>
              <Check size={15} className="ob-new" />Commit{dirty ? ` (${changeCount(pending)})` : ""}
            </button>
            <button className="obj-btn" disabled={!dirty || committing} onClick={reset}><Undo2 size={15} className="ob-delete" />Discard</button>
          </>
        )}
        <button className="obj-btn" title="Export this table" onClick={() => openTransfer({ kind: "export", connectionId: tab.connectionId, database: tab.database, source: { kind: "table", table: tab.name }, label: tab.name })}>
          <Upload size={15} className="ob-design" />Export
        </button>
        {!readOnly && (
          <button className="obj-btn" title="Import a CSV file" onClick={() => openTransfer({ kind: "import", connectionId: tab.connectionId, database: tab.database, table: tab.name })}>
            <Download size={15} className="ob-new" />Import
          </button>
        )}
        {tab.objectKind === "table" && (
          <button className="obj-btn" title="Fill the table with realistic test data" onClick={() => openTransfer({ kind: "generate", connectionId: tab.connectionId, database: tab.database, table: tab.name })}>
            <Sparkles size={15} className="ob-design" />Generate
          </button>
        )}
        <button className={`obj-btn${showFilters || filters.length ? " active-filter" : ""}`} onClick={() => setShowFilters((v) => !v)}>
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
                <select className="input sm" value={d.column} aria-label="Filter column" onChange={(e) => patch({ column: e.target.value })}>
                  {columns.filter((c) => !c.hidden).map((c) => <option key={c.name} value={c.name}>{c.name}</option>)}
                </select>
                <select className="input sm" value={d.op} aria-label="Filter operator" onChange={(e) => patch({ op: e.target.value as FilterOp })}>
                  {OPS.map((o) => <option key={o.op} value={o.op}>{o.label}</option>)}
                </select>
                {needsValue && (
                  <input className="input sm" value={d.value} aria-label="Filter value" placeholder="value" onChange={(e) => patch({ value: e.target.value })} onKeyDown={(e) => e.key === "Enter" && void applyFilters()} />
                )}
                <button className="icon-btn" aria-label="Remove filter" onClick={() => setDrafts((ds) => ds.filter((_, j) => j !== i))}><X size={14} /></button>
              </div>
            );
          })}
          <div className="filter-actions">
            <button className="btn sm" onClick={() => setDrafts((ds) => [...ds, { column: columns.find((c) => !c.hidden)?.name ?? "", op: "eq", value: "" }])}><Plus size={13} />Add condition</button>
            <button className="btn sm primary" onClick={applyFilters}>Apply</button>
            {filters.length > 0 && <button className="btn sm ghost" onClick={async () => { if (!(await guard())) return; reset(); setDrafts([]); setFilters([]); setOffset(0); }}>Clear</button>}
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
          onSort={toggleSort}
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
          onCellActivate={startEdit}
          onCellEdit={(r, column) => startEdit(r, column, true)}
          onAppendRow={readOnly || lastRowUntouched ? undefined : appendRow}
          renderEditor={(r, column) =>
            editing?.row === r && editing.column === column ? (
              <CellEditor
                kind={dateKind(columns[colIndex(column)]?.typeName ?? "")}
                initial={cellValue(pending, refOf(r), column, r < saved ? pageRows[r][colIndex(column)] : undefined)}
                onDone={(v) => {
                  if (v) write(r, column, v);
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
          <button className="icon-btn" title="First page" aria-label="First page" disabled={offset === 0} onClick={() => go(0)}><ChevronsLeft size={16} /></button>
          <button className="icon-btn" title="Previous page" aria-label="Previous page" disabled={offset === 0} onClick={() => go(offset - pageSize)}><ChevronLeft size={16} /></button>
          <span className="pager-text">{formatCount(saved ? offset + 1 : 0)}–{formatCount(offset + saved)} of {total === undefined ? "…" : formatCount(total)}</span>
          <button className="icon-btn" title="Next page" aria-label="Next page" disabled={total === undefined || offset + pageSize >= total} onClick={() => go(offset + pageSize)}><ChevronRight size={16} /></button>
          <button className="icon-btn" title="Last page" aria-label="Last page" disabled={total === undefined || offset >= lastOffset} onClick={() => go(lastOffset)}><ChevronsRight size={16} /></button>
          <select className="input sm" value={pageSize} aria-label="Rows per page" onChange={async (e) => { const n = Number(e.target.value); if (!(await guard())) return; reset(); setPageSize(n); setOffset(0); }}>
            {PAGE_SIZES.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        </div>
        <button className="icon-btn" title="Refresh" aria-label="Refresh" onClick={refresh}><RefreshCw size={15} className={page.isFetching ? "spin" : undefined} /></button>
      </div>
    </div>
  );
}

function CellEditor({ initial, kind, onDone }: { initial: CellValue; kind: DateKind | null; onDone: (v: CellValue | null) => void }) {
  const start = initial.type === "null" ? "" : initial.type === "bytes" ? "" : String(initial.value);
  const [text, setText] = useState(start);
  const [pickerAt, setPickerAt] = useState<DOMRect | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const picker = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  // Unchanged text is not an edit (so Enter on a NULL cell keeps it NULL).
  const commit = (value: string) => onDone(value !== start ? { type: "text", value } : null);
  /** Focus leaving both the editor and its picker ends the edit, saving it. */
  const onBlur = (e: FocusEvent) => {
    const to = e.relatedTarget as Node | null;
    if (to && (wrap.current?.contains(to) || picker.current?.contains(to))) return;
    commit(text);
  };
  // Keys go to the picker while it is open (Esc closes it, Enter picks); the editor gets them back on Cancel.
  const openPicker = () => {
    setPickerAt(wrap.current?.getBoundingClientRect() ?? null);
    requestAnimationFrame(() => picker.current?.focus());
  };
  return (
    <div className="cell-editor-wrap" ref={wrap}>
      <input
        ref={input}
        className="cell-editor"
        autoFocus
        value={text}
        placeholder={initial.type === "null" ? "NULL" : undefined}
        onChange={(e) => setText(e.target.value)}
        onBlur={onBlur}
        onKeyDown={(e) => {
          if (kind && e.altKey && e.key === "ArrowDown") {
            e.preventDefault();
            openPicker();
            return;
          }
          // Tab also saves; the grid then moves the editor to the neighbouring cell.
          if (e.key === "Enter" || e.key === "Tab") commit(text);
          if (e.key === "Escape") onDone(null);
        }}
      />
      {kind && (
        <button
          className="cell-editor-btn"
          tabIndex={-1}
          title="Pick a date (Alt+↓)"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => (pickerAt ? setPickerAt(null) : openPicker())}
        >
          …
        </button>
      )}
      {kind && pickerAt && (
        <DatePicker
          ref={picker}
          kind={kind}
          value={text}
          anchor={pickerAt}
          onBlur={onBlur}
          onPick={(value) => {
            // The editor goes away with the pick; keep the keyboard in the grid.
            wrap.current?.closest<HTMLElement>(".grid")?.focus({ preventScroll: true });
            commit(value);
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
