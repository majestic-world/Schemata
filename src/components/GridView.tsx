import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { getCoreRowModel, useReactTable, type ColumnDef } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp } from "lucide-react";
import type { CellValue, ColumnMeta, SortSpec } from "../api/types";
import { displayCell, toTsv } from "../lib/cells";
import { notify } from "../store/toasts";

const ROW_HEIGHT = 26;
const HEADER_HEIGHT = 28;

export type GridSelection = { rows: number[]; columns: string[] };

type Props = {
  columns: ColumnMeta[];
  rows: CellValue[][];
  /** Rows at or beyond this index are new (unsaved) rows and are numbered `*`. */
  savedRowCount?: number;
  rowNumberOffset?: number;
  sort?: SortSpec | null;
  onSort?: (column: string) => void;
  cellValue?: (row: number, column: string, original: CellValue) => CellValue;
  rowClass?: (row: number) => string | undefined;
  cellClass?: (row: number, column: string) => string | undefined;
  /** Text shown instead of the value, dimmed like NULL (e.g. `(Default)` for unset new-row columns). */
  cellPlaceholder?: (row: number, column: string) => string | undefined;
  /** Double-click or Enter on a cell. */
  onCellActivate?: (row: number, column: string) => void;
  /** Plain click on a cell, or Tab onto it: open its editor if the cell is editable, silently skip otherwise. */
  onCellEdit?: (row: number, column: string) => void;
  renderEditor?: (row: number, column: string) => ReactNode | null;
  /**
   * Appends a new row, starting to edit `column` when given. When set, ArrowDown on the last row calls it, and an
   * empty grid shows a `(N/A)` placeholder row that turns into a new row on ArrowDown, Enter or double-click.
   */
  onAppendRow?: (column?: string) => void;
  onSelectionChange?: (s: GridSelection) => void;
  /** Stages deletion of the selected loaded rows; omitted for read-only grids. */
  onDeleteRows?: (rows: number[]) => void;
  /** Changes when the underlying data is replaced, clearing the selection and scroll. */
  resetKey?: unknown;
  emptyText?: string;
};

type Pos = { r: number; c: number };

function widthFor(c: ColumnMeta): number {
  const t = c.typeName.toLowerCase();
  if (/bigint/.test(t)) return 200;
  if (/int|bit|bool|year/.test(t)) return 110;
  if (/date|time/.test(t)) return 180;
  if (/decimal|numeric|float|double|real/.test(t)) return 140;
  return Math.min(320, Math.max(140, c.name.length * 9 + 40));
}

export function GridView(props: Props) {
  const { columns: meta, rows, savedRowCount = rows.length, rowNumberOffset = 0 } = props;
  const [anchor, setAnchor] = useState<Pos | null>(null);
  const [focus, setFocus] = useState<Pos | null>(null);
  const dragging = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  const visible = useMemo(() => meta.map((c, idx) => ({ c, idx })).filter((x) => !x.c.hidden), [meta]);
  const placeholderRow = rows.length === 0 && !!props.onAppendRow && visible.length > 0;
  const rowCount = placeholderRow ? 1 : rows.length;
  const columns = useMemo<ColumnDef<CellValue[]>[]>(
    () => visible.map(({ c, idx }) => ({ id: c.name, header: c.name, size: widthFor(c), accessorFn: (row) => row[idx], meta: c })),
    [visible],
  );
  const table = useReactTable({ data: rows, columns, getCoreRowModel: getCoreRowModel(), columnResizeMode: "onChange", defaultColumn: { minSize: 60, maxSize: 900 } });
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    scrollMargin: HEADER_HEIGHT,
  });

  useEffect(() => {
    setAnchor(null);
    setFocus(null);
    scrollRef.current?.scrollTo({ top: 0 });
  }, [props.resetKey]);
  useEffect(() => {
    const up = () => (dragging.current = false);
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, []);

  const rect = anchor && focus
    ? { r0: Math.min(anchor.r, focus.r), r1: Math.max(anchor.r, focus.r), c0: Math.min(anchor.c, focus.c), c1: Math.max(anchor.c, focus.c) }
    : null;
  const { onSelectionChange } = props;
  useEffect(() => {
    if (!onSelectionChange) return;
    if (!rect) return onSelectionChange({ rows: [], columns: [] });
    const rs: number[] = [];
    for (let r = rect.r0; r <= Math.min(rect.r1, rows.length - 1); r++) rs.push(r);
    onSelectionChange({ rows: rs, columns: visible.slice(rect.c0, rect.c1 + 1).map((v) => v.c.name) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rect?.r0, rect?.r1, rect?.c0, rect?.c1, visible, onSelectionChange, rows.length]);

  const valueAt = (r: number, c: number): CellValue => {
    const col = visible[c];
    const original = rows[r][col.idx];
    return props.cellValue ? props.cellValue(r, col.c.name, original) : original;
  };
  const copy = async () => {
    if (!rect) return;
    const out: CellValue[][] = [];
    for (let r = rect.r0; r <= Math.min(rect.r1, rows.length - 1); r++) {
      const line: CellValue[] = [];
      for (let c = rect.c0; c <= rect.c1; c++) line.push(valueAt(r, c));
      out.push(line);
    }
    await navigator.clipboard.writeText(toTsv(out));
    const n = out.length * (rect.c1 - rect.c0 + 1);
    notify(`Copied ${n} cell${n === 1 ? "" : "s"}`);
  };

  const headers = table.getHeaderGroups()[0]?.headers ?? [];
  const totalWidth = 50 + headers.reduce((s, h) => s + h.getSize(), 0);

  /** Cell a navigation key moves the focus to; null when the key does not navigate. */
  const moveTarget = (e: KeyboardEvent): Pos | null => {
    if (rowCount === 0 || visible.length === 0) return null;
    const lastRow = rowCount - 1;
    const lastCol = visible.length - 1;
    const page = Math.max(1, Math.floor(((scrollRef.current?.clientHeight ?? 0) - HEADER_HEIGHT) / ROW_HEIGHT) - 1);
    const mod = e.ctrlKey || e.metaKey;
    const at = (r: number, c: number): Pos => ({ r: Math.min(lastRow, Math.max(0, r)), c: Math.min(lastCol, Math.max(0, c)) });
    if (!focus) return ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"].includes(e.key) ? at(0, 0) : null;
    switch (e.key) {
      case "ArrowUp": return at(mod ? 0 : focus.r - 1, focus.c);
      case "ArrowDown": return at(mod ? lastRow : focus.r + 1, focus.c);
      case "ArrowLeft": return at(focus.r, mod ? 0 : focus.c - 1);
      case "ArrowRight": return at(focus.r, mod ? lastCol : focus.c + 1);
      case "Home": return at(mod ? 0 : focus.r, 0);
      case "End": return at(mod ? lastRow : focus.r, lastCol);
      case "PageUp": return at(focus.r - page, focus.c);
      case "PageDown": return at(focus.r + page, focus.c);
      default: return null;
    }
  };
  /** Scrolls just enough to show a cell below the sticky header and right of the row numbers. */
  const scrollIntoView = ({ r, c }: Pos) => {
    const el = scrollRef.current;
    if (!el) return;
    const top = HEADER_HEIGHT + r * ROW_HEIGHT;
    if (top - HEADER_HEIGHT < el.scrollTop) el.scrollTop = top - HEADER_HEIGHT;
    else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
    const left = 50 + headers.slice(0, c).reduce((s, h) => s + h.getSize(), 0);
    const right = left + headers[c].getSize();
    if (left - 50 < el.scrollLeft) el.scrollLeft = left - 50;
    else if (right > el.scrollLeft + el.clientWidth) el.scrollLeft = right - el.clientWidth;
  };

  return (
    <div
      className="grid"
      ref={scrollRef}
      tabIndex={0}
      onKeyDown={(e) => {
        // Tab moves to the next cell of the row, wrapping to the first (Shift+Tab: backwards), and opens it for
        // editing; an open editor saves first. Cells that cannot be edited (binary) are only selected.
        if (e.key === "Tab" && focus && visible.length > 0 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          e.preventDefault();
          const n = visible.length;
          const next = { r: focus.r, c: (focus.c + (e.shiftKey ? n - 1 : 1)) % n };
          setAnchor(next);
          setFocus(next);
          scrollIntoView(next);
          props.onCellEdit?.(next.r, visible[next.c].c.name);
          // A cell that cannot be edited opens no editor; keep the keyboard in the grid.
          requestAnimationFrame(() => !scrollRef.current?.contains(document.activeElement) && scrollRef.current?.focus({ preventScroll: true }));
          return;
        }
        if (e.target instanceof Element && e.target.closest("input, textarea, select, [contenteditable]")) {
          // Closing the cell editor removes the focused input; give focus back so navigation continues.
          if (e.key === "Enter" || e.key === "Escape") requestAnimationFrame(() => scrollRef.current?.focus({ preventScroll: true }));
          return;
        }
        if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "a") {
          e.preventDefault();
          if (rows.length > 0 && visible.length > 0) {
            setAnchor({ r: 0, c: 0 });
            setFocus({ r: rows.length - 1, c: visible.length - 1 });
          }
          return;
        }
        if (e.key === "Delete" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && props.onDeleteRows && rect) {
          e.preventDefault();
          if (e.repeat) return;
          const selected: number[] = [];
          for (let r = rect.r0; r <= Math.min(rect.r1, rows.length - 1); r++) selected.push(r);
          props.onDeleteRows(selected);
          setAnchor(null);
          setFocus(null);
          return;
        }
        if (e.key === "Enter" && focus && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
          e.preventDefault();
          const name = visible[focus.c].c.name;
          if (placeholderRow) props.onAppendRow?.(name);
          else props.onCellActivate?.(focus.r, name);
          return;
        }
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
          e.preventDefault();
          void copy();
          return;
        }
        // Like Navicat, moving down from the last row (or the placeholder row) adds a new row.
        if (e.key === "ArrowDown" && props.onAppendRow && focus && focus.r === rowCount - 1 && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          props.onAppendRow();
          const next = { r: rows.length, c: focus.c };
          setAnchor(next);
          setFocus(next);
          scrollIntoView(next);
          return;
        }
        const next = moveTarget(e);
        if (!next) return;
        e.preventDefault();
        if (e.shiftKey && anchor) setFocus(next);
        else {
          setAnchor(next);
          setFocus(next);
        }
        scrollIntoView(next);
      }}
    >
      <div className="grid-header" style={{ width: totalWidth, height: HEADER_HEIGHT }}>
        <div className="grid-rownum head" />
        {headers.map((h) => {
          const c = h.column.columnDef.meta as ColumnMeta;
          return (
            <div
              key={h.id}
              className={`grid-th${props.onSort ? " sortable" : ""}`}
              style={{ width: h.getSize() }}
              onClick={() => props.onSort?.(c.name)}
              title={`${c.name} ${c.typeName}`}
            >
              <span className="th-name">{c.name}</span>
              {props.sort?.column === c.name && (props.sort.desc ? <ArrowDown size={13} /> : <ArrowUp size={13} />)}
              <span className="th-type">{c.typeName}</span>
              <span className={`resizer${h.column.getIsResizing() ? " on" : ""}`} onMouseDown={h.getResizeHandler()} onClick={(e) => e.stopPropagation()} />
            </div>
          );
        })}
      </div>
      <div style={{ height: virtualizer.getTotalSize(), width: totalWidth, position: "relative" }}>
        {virtualizer.getVirtualItems().map((vr) => {
          const r = vr.index;
          return (
            <div
              key={vr.key}
              className={`grid-row${r % 2 ? " odd" : ""} ${props.rowClass?.(r) ?? ""}`}
              style={{ transform: `translateY(${vr.start - HEADER_HEIGHT}px)`, height: ROW_HEIGHT }}
            >
              <div className="grid-rownum">{r < savedRowCount ? rowNumberOffset + r + 1 : "*"}</div>
              {headers.map((h, c) => {
                const name = visible[c].c.name;
                const editor = props.renderEditor?.(r, name);
                const placeholder = props.cellPlaceholder?.(r, name);
                const d = placeholder !== undefined ? { text: placeholder, tone: "null" } : displayCell(valueAt(r, c));
                const selected = rect && r >= rect.r0 && r <= rect.r1 && c >= rect.c0 && c <= rect.c1;
                return (
                  <div
                    key={h.id}
                    className={`grid-cell ${d.tone}${selected ? " selected" : ""}${editor ? " editing" : ""} ${props.cellClass?.(r, name) ?? ""}`}
                    style={{ width: h.getSize() }}
                    title={d.text.length > 40 ? d.text : undefined}
                    data-cell={`${r}:${name}`}
                    onMouseDown={(e) => {
                      if (e.button !== 0 || editor) return;
                      if (e.shiftKey && anchor) setFocus({ r, c });
                      else {
                        setAnchor({ r, c });
                        setFocus({ r, c });
                      }
                      dragging.current = true;
                    }}
                    onMouseEnter={() => dragging.current && setFocus({ r, c })}
                    onClick={(e) => {
                      if (!editor && !e.shiftKey && !e.ctrlKey && !e.metaKey) props.onCellEdit?.(r, name);
                    }}
                    onDoubleClick={() => props.onCellActivate?.(r, name)}
                  >
                    {editor ?? <span className="cell-text">{d.text}</span>}
                  </div>
                );
              })}
            </div>
          );
        })}
        {placeholderRow && (
          <div className="grid-row" style={{ height: ROW_HEIGHT }}>
            <div className="grid-rownum">*</div>
            {headers.map((h, c) => (
              <div
                key={h.id}
                className={`grid-cell null${focus?.r === 0 && focus.c === c ? " selected" : ""}`}
                style={{ width: h.getSize() }}
                onMouseDown={(e) => {
                  if (e.button !== 0) return;
                  setAnchor({ r: 0, c });
                  setFocus({ r: 0, c });
                }}
                onDoubleClick={() => props.onAppendRow?.(visible[c].c.name)}
              >
                <span className="cell-text">(N/A)</span>
              </div>
            ))}
          </div>
        )}
      </div>
      {rows.length === 0 && !placeholderRow && props.emptyText && <div className="empty grid-empty"><p>{props.emptyText}</p></div>}
    </div>
  );
}
