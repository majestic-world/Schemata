import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { getCoreRowModel, useReactTable, type ColumnDef } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Filter as FilterIcon, Plus, RefreshCw, X } from "lucide-react";
import { api } from "../api/client";
import type { CellValue, ColumnMeta, Filter, FilterOp, SortSpec } from "../api/types";
import { useTabs, type TableTab } from "../store/tabs";
import { displayCell, toTsv } from "../lib/cells";
import { formatCount } from "../lib/format";
import { notify } from "../store/toasts";

const ROW_HEIGHT = 30;
const HEADER_HEIGHT = 36;
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

type DraftFilter = { column: string; op: FilterOp; value: string };
type Pos = { r: number; c: number };

/** Hooks used by the editing layer to decorate and react to cells. */
export type GridDecorations = {
  rowClass?: (rowIndex: number) => string | undefined;
  cellValue?: (rowIndex: number, column: string, original: CellValue) => CellValue;
  cellClass?: (rowIndex: number, column: string) => string | undefined;
  onCellDoubleClick?: (rowIndex: number, column: string) => void;
  renderEditor?: (rowIndex: number, column: string) => ReactNode | null;
  extraRows?: CellValue[][];
  toolbar?: ReactNode;
};

function widthFor(c: ColumnMeta): number {
  const t = c.typeName.toLowerCase();
  if (/bigint/.test(t)) return 200;
  if (/int|bit|bool|year/.test(t)) return 110;
  if (/date|time/.test(t)) return 180;
  if (/decimal|numeric|float|double|real/.test(t)) return 140;
  return Math.min(320, Math.max(140, c.name.length * 9 + 40));
}

export function DataGrid({ tab, decorations }: { tab: TableTab; decorations?: GridDecorations }) {
  const qc = useQueryClient();
  const setRowCount = useTabs((s) => s.setRowCount);
  const [pageSize, setPageSize] = useState(500);
  const [offset, setOffset] = useState(0);
  const [sort, setSort] = useState<SortSpec | null>(null);
  const [filters, setFilters] = useState<Filter[]>([]);
  const [drafts, setDrafts] = useState<DraftFilter[]>([]);
  const [showFilters, setShowFilters] = useState(false);
  const [anchor, setAnchor] = useState<Pos | null>(null);
  const [focus, setFocus] = useState<Pos | null>(null);
  const dragging = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

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

  const meta = page.data?.columns ?? [];
  const visible = useMemo(() => meta.map((c, idx) => ({ c, idx })).filter((x) => !x.c.hidden), [meta]);
  const rows = useMemo(() => [...(page.data?.rows ?? []), ...(decorations?.extraRows ?? [])], [page.data, decorations?.extraRows]);

  const columns = useMemo<ColumnDef<CellValue[]>[]>(
    () => visible.map(({ c, idx }) => ({ id: c.name, header: c.name, size: widthFor(c), accessorFn: (row) => row[idx], meta: c })),
    [visible],
  );
  const table = useReactTable({
    data: rows,
    columns,
    getCoreRowModel: getCoreRowModel(),
    columnResizeMode: "onChange",
    defaultColumn: { minSize: 60, maxSize: 900 },
  });

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    scrollMargin: HEADER_HEIGHT,
  });

  useEffect(() => {
    const up = () => (dragging.current = false);
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, []);

  const total = count.data;
  const lastOffset = total !== undefined ? Math.max(0, Math.floor((total - 1) / pageSize) * pageSize) : offset;
  const go = (o: number) => {
    setOffset(Math.max(0, o));
    setAnchor(null);
    setFocus(null);
    scrollRef.current?.scrollTo({ top: 0 });
  };
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["page", ...base] });
    void qc.invalidateQueries({ queryKey: ["count", ...base] });
  };
  const toggleSort = (column: string) => {
    setSort((s) => (s?.column !== column ? { column, desc: false } : !s.desc ? { column, desc: true } : null));
    go(0);
  };
  const applyFilters = () => {
    const next: Filter[] = drafts
      .filter((d) => d.column)
      .map((d) => ({
        column: d.column,
        op: d.op,
        value: OPS.find((o) => o.op === d.op)!.needsValue ? { type: "text", value: d.value } : null,
      }));
    setFilters(next);
    go(0);
  };

  const rect = anchor && focus
    ? { r0: Math.min(anchor.r, focus.r), r1: Math.max(anchor.r, focus.r), c0: Math.min(anchor.c, focus.c), c1: Math.max(anchor.c, focus.c) }
    : null;
  const valueAt = (r: number, c: number): CellValue => {
    const col = visible[c];
    const original = rows[r][col.idx];
    return decorations?.cellValue ? decorations.cellValue(r, col.c.name, original) : original;
  };
  const copy = async () => {
    if (!rect) return;
    const out: CellValue[][] = [];
    for (let r = rect.r0; r <= rect.r1; r++) {
      const line: CellValue[] = [];
      for (let c = rect.c0; c <= rect.c1; c++) line.push(valueAt(r, c));
      out.push(line);
    }
    await navigator.clipboard.writeText(toTsv(out));
    notify(`Copied ${out.length * (rect.c1 - rect.c0 + 1)} cell${out.length * (rect.c1 - rect.c0 + 1) === 1 ? "" : "s"}`);
  };

  const headers = table.getHeaderGroups()[0]?.headers ?? [];
  const totalWidth = 56 + headers.reduce((s, h) => s + h.getSize(), 0);
  const items = virtualizer.getVirtualItems();
  const from = rows.length ? offset + 1 : 0;
  const to = offset + (page.data?.rows.length ?? 0);

  return (
    <div className="grid-wrap">
      <div className="action-bar grid-toolbar">
        {decorations?.toolbar}
        <button className={`btn${showFilters || filters.length ? " active-filter" : ""}`} onClick={() => setShowFilters((v) => !v)}>
          <FilterIcon size={15} />Filter{filters.length ? ` (${filters.length})` : ""}
        </button>
        <div className="grow" />
        <div className="pager">
          <button className="icon-btn" title="First page" disabled={offset === 0} onClick={() => go(0)}><ChevronsLeft size={16} /></button>
          <button className="icon-btn" title="Previous page" disabled={offset === 0} onClick={() => go(offset - pageSize)}><ChevronLeft size={16} /></button>
          <span className="pager-text">{formatCount(from)}–{formatCount(to)} of {total === undefined ? "…" : formatCount(total)}</span>
          <button className="icon-btn" title="Next page" disabled={total === undefined || offset + pageSize >= total} onClick={() => go(offset + pageSize)}><ChevronRight size={16} /></button>
          <button className="icon-btn" title="Last page" disabled={total === undefined || offset >= lastOffset} onClick={() => go(lastOffset)}><ChevronsRight size={16} /></button>
          <select className="input sm" value={pageSize} onChange={(e) => { setPageSize(Number(e.target.value)); go(0); }} aria-label="Rows per page">
            {PAGE_SIZES.map((n) => <option key={n} value={n}>{n} / page</option>)}
          </select>
        </div>
        <button className="icon-btn" title="Refresh" onClick={refresh}><RefreshCw size={15} className={page.isFetching ? "spin" : undefined} /></button>
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
                  {visible.map(({ c }) => <option key={c.name} value={c.name}>{c.name}</option>)}
                </select>
                <select className="input sm" value={d.op} aria-label="Filter operator" onChange={(e) => patch({ op: e.target.value as FilterOp })}>
                  {OPS.map((o) => <option key={o.op} value={o.op}>{o.label}</option>)}
                </select>
                {needsValue && (
                  <input className="input sm" value={d.value} aria-label="Filter value" placeholder="value" onChange={(e) => patch({ value: e.target.value })} onKeyDown={(e) => e.key === "Enter" && applyFilters()} />
                )}
                <button className="icon-btn" aria-label="Remove filter" onClick={() => setDrafts((ds) => ds.filter((_, j) => j !== i))}><X size={14} /></button>
              </div>
            );
          })}
          <div className="filter-actions">
            <button className="btn sm" onClick={() => setDrafts((ds) => [...ds, { column: visible[0]?.c.name ?? "", op: "eq", value: "" }])}><Plus size={13} />Add condition</button>
            <button className="btn sm primary" onClick={applyFilters}>Apply</button>
            {filters.length > 0 && <button className="btn sm ghost" onClick={() => { setDrafts([]); setFilters([]); go(0); }}>Clear</button>}
          </div>
        </div>
      )}
      {page.isError ? (
        <div className="empty"><p className="error-text">{(page.error as { message: string }).message}</p></div>
      ) : (
        <div
          className="grid"
          ref={scrollRef}
          tabIndex={0}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
              e.preventDefault();
              void copy();
            }
          }}
        >
          <div className="grid-header" style={{ width: totalWidth, height: HEADER_HEIGHT }}>
            <div className="grid-rownum head" />
            {headers.map((h) => {
              const c = h.column.columnDef.meta as ColumnMeta;
              return (
                <div key={h.id} className="grid-th" style={{ width: h.getSize() }} onClick={() => toggleSort(c.name)} title={`${c.name} ${c.typeName}`}>
                  <span className="th-name">{c.name}</span>
                  {sort?.column === c.name && (sort.desc ? <ArrowDown size={13} /> : <ArrowUp size={13} />)}
                  <span className="th-type">{c.typeName}</span>
                  <span
                    className={`resizer${h.column.getIsResizing() ? " on" : ""}`}
                    onMouseDown={h.getResizeHandler()}
                    onClick={(e) => e.stopPropagation()}
                  />
                </div>
              );
            })}
          </div>
          <div style={{ height: virtualizer.getTotalSize(), width: totalWidth, position: "relative" }}>
            {items.map((vr) => {
              const r = vr.index;
              return (
                <div
                  key={vr.key}
                  className={`grid-row${r % 2 ? " odd" : ""} ${decorations?.rowClass?.(r) ?? ""}`}
                  style={{ transform: `translateY(${vr.start - HEADER_HEIGHT}px)`, height: ROW_HEIGHT }}
                >
                  <div className="grid-rownum">{r < (page.data?.rows.length ?? 0) ? offset + r + 1 : "*"}</div>
                  {headers.map((h, c) => {
                    const name = visible[c].c.name;
                    const editor = decorations?.renderEditor?.(r, name);
                    const v = valueAt(r, c);
                    const d = displayCell(v);
                    const selected = rect && r >= rect.r0 && r <= rect.r1 && c >= rect.c0 && c <= rect.c1;
                    return (
                      <div
                        key={h.id}
                        className={`grid-cell ${d.tone}${selected ? " selected" : ""} ${decorations?.cellClass?.(r, name) ?? ""}`}
                        style={{ width: h.getSize() }}
                        title={d.text.length > 40 ? d.text : undefined}
                        onMouseDown={(e) => {
                          if (e.button !== 0) return;
                          if (e.shiftKey && anchor) setFocus({ r, c });
                          else {
                            setAnchor({ r, c });
                            setFocus({ r, c });
                          }
                          dragging.current = true;
                        }}
                        onMouseEnter={() => dragging.current && setFocus({ r, c })}
                        onDoubleClick={() => decorations?.onCellDoubleClick?.(r, name)}
                      >
                        {editor ?? <span className="cell-text">{d.text}</span>}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
          {page.isSuccess && rows.length === 0 && <div className="empty grid-empty"><p>No rows{filters.length ? " match the filter" : ""}.</p></div>}
        </div>
      )}
    </div>
  );
}
