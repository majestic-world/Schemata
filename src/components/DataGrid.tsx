import { useCallback, useEffect, useMemo, useState } from "react";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Upload, Ban, Check, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Filter as FilterIcon, Lock, Plus, RefreshCw, Trash2, Undo2, X } from "lucide-react";
import { api } from "../api/client";
import type { CellValue, ChangeError, Filter, FilterOp, SortSpec } from "../api/types";
import { useTabs, type TableTab } from "../store/tabs";
import { formatCount } from "../lib/format";
import { notify, notifyError } from "../store/toasts";
import { confirmAction } from "../store/confirm";
import { useGridState, type DraftFilter } from "../store/gridState";
import { openTransfer } from "../store/transfer";
import { MOD, isMod } from "../lib/platform";
import { GridView, type GridSelection } from "./GridView";
import { useTableInfo } from "./TableTabView";
import {
  addRow,
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

const PAGE_SIZES = [100, 500, 1000];

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

  const startEdit = useCallback(
    (row: number, column: string) => {
      if (readOnly) {
        notifyError("This data is read-only", { message: "The table has no primary or unique key, so rows cannot be addressed safely." });
        return;
      }
      const v = original(row, column);
      if (v.type === "bytes" && row < saved) {
        notifyError("Binary values cannot be edited here", { message: "Set the cell to NULL or use the SQL editor." });
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

  const commit = async () => {
    if (!identity || !data) return;
    const identityColumns = identity.kind === "primaryKey" || identity.kind === "uniqueKey" ? identity.columns : identity.kind === "rowId" ? ["__rowid__"] : [];
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
          info.data && <span className="badge"><Lock size={13} />Read-only — {info.data.kind === "view" ? "views are not editable" : "no primary or unique key"}</span>
        ) : (
          <>
            <button className="btn" onClick={() => { const r = addRow(pending); setPending(r.pending); setEditing({ row: saved + r.row.index, column: columns.find((c) => !c.hidden)?.name ?? "" }); }}>
              <Plus size={15} />Add Row
            </button>
            <button className="btn" disabled={selection.rows.length === 0} onClick={() => { setPending((p) => deleteRows(p, selection.rows.map(refOf))); setEditing(null); }}>
              <Trash2 size={15} />Delete
            </button>
            <button className="btn" disabled={selection.rows.length === 0} title="Set selected cells to NULL" onClick={() => selection.rows.forEach((r) => selection.columns.forEach((c) => write(r, c, { type: "null" })))}>
              <Ban size={15} />Set NULL
            </button>
            <div className="toolbar-sep" />
            <button className="btn primary" disabled={!dirty || committing} onClick={commit} title={`Commit (${MOD}+S)`}>
              <Check size={15} />Commit{dirty ? ` (${changeCount(pending)})` : ""}
            </button>
            <button className="btn" disabled={!dirty || committing} onClick={reset}><Undo2 size={15} />Discard</button>
          </>
        )}
        <button className="btn" title="Export this table" onClick={() => openTransfer({ kind: "export", connectionId: tab.connectionId, database: tab.database, source: { kind: "table", table: tab.name }, label: tab.name })}>
          <Upload size={15} />Export
        </button>
        {!readOnly && (
          <button className="btn" title="Import a CSV file" onClick={() => openTransfer({ kind: "import", connectionId: tab.connectionId, database: tab.database, table: tab.name })}>
            <Download size={15} />Import
          </button>
        )}
        <button className={`btn${showFilters || filters.length ? " active-filter" : ""}`} onClick={() => setShowFilters((v) => !v)}>
          <FilterIcon size={15} />Filter{filters.length ? ` (${filters.length})` : ""}
        </button>
        <div className="grow" />
        <div className="pager">
          <button className="icon-btn" title="First page" aria-label="First page" disabled={offset === 0} onClick={() => go(0)}><ChevronsLeft size={16} /></button>
          <button className="icon-btn" title="Previous page" aria-label="Previous page" disabled={offset === 0} onClick={() => go(offset - pageSize)}><ChevronLeft size={16} /></button>
          <span className="pager-text">{formatCount(saved ? offset + 1 : 0)}–{formatCount(offset + saved)} of {total === undefined ? "…" : formatCount(total)}</span>
          <button className="icon-btn" title="Next page" aria-label="Next page" disabled={total === undefined || offset + pageSize >= total} onClick={() => go(offset + pageSize)}><ChevronRight size={16} /></button>
          <button className="icon-btn" title="Last page" aria-label="Last page" disabled={total === undefined || offset >= lastOffset} onClick={() => go(lastOffset)}><ChevronsRight size={16} /></button>
          <select className="input sm" value={pageSize} aria-label="Rows per page" onChange={async (e) => { const n = Number(e.target.value); if (!(await guard())) return; reset(); setPageSize(n); setOffset(0); }}>
            {PAGE_SIZES.map((n) => <option key={n} value={n}>{n} / page</option>)}
          </select>
        </div>
        <button className="icon-btn" title="Refresh" aria-label="Refresh" onClick={refresh}><RefreshCw size={15} className={page.isFetching ? "spin" : undefined} /></button>
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
          columns={columns}
          rows={rows}
          savedRowCount={saved}
          rowNumberOffset={offset}
          sort={sort}
          onSort={toggleSort}
          resetKey={data}
          onSelectionChange={setSelection}
          emptyText={page.isSuccess ? `No rows${filters.length ? " match the filter" : ""}.` : undefined}
          cellValue={(r, column, orig) => cellValue(pending, refOf(r), column, r < saved ? orig : undefined)}
          rowClass={(r) => {
            const ref = refOf(r);
            if (sameRef(failedRow, ref)) return "row-failed";
            if (ref.kind === "new") return "row-new";
            return isDeleted(pending, ref) ? "row-deleted" : undefined;
          }}
          cellClass={(r, column) => (isEdited(pending, refOf(r), column) ? "cell-edited" : undefined)}
          onCellDoubleClick={startEdit}
          renderEditor={(r, column) =>
            editing?.row === r && editing.column === column ? (
              <CellEditor
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
    </div>
  );
}

function CellEditor({ initial, onDone }: { initial: CellValue; onDone: (v: CellValue | null) => void }) {
  const start = initial.type === "null" ? "" : initial.type === "bytes" ? "" : String(initial.value);
  const [text, setText] = useState(start);
  // Unchanged text is not an edit (so Enter on a NULL cell keeps it NULL).
  const finish = (save: boolean) => onDone(save && text !== start ? { type: "text", value: text } : null);
  return (
    <input
      className="cell-editor"
      autoFocus
      value={text}
      placeholder={initial.type === "null" ? "NULL" : undefined}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => finish(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(true);
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}
