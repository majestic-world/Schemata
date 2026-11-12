import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Copy, KeyRound, Plus, Save, Trash2, Undo2 } from "lucide-react";
import { api } from "../api/client";
import type { ColumnDesign, Engine, ForeignKeyDesign, IndexDesign, IndexKind, IndexMethod, TableDesign } from "../api/types";
import { FK_ACTIONS, MARIADB_ENGINES, MYSQL_ENGINES, composeType, isNumericType, parseType, retype, typeSuggestions } from "../lib/columnTypes";
import { notify, notifyError } from "../store/toasts";
import { useDesignerPrefs } from "../store/designerPrefs";
import { canMoveColumns, moveColumns, removeColumns, renameColumn, setPrimaryKey, useDesignerDrafts } from "../store/designerDrafts";
import { confirmAction } from "../store/confirm";
import { hasPendingEdits, useGridStates } from "../store/gridState";
import { MOD, isMod } from "../lib/platform";
import { Dialog } from "./Dialog";
import { ComboInput } from "./ComboInput";

type Section = "columns" | "indexes" | "foreignKeys" | "options" | "ddl";

/** Index types each engine supports; SQLite has no FULLTEXT/SPATIAL indexes, methods or comments. */
const INDEX_KINDS: Record<"sqlite" | "mysql", IndexKind[]> = { sqlite: ["normal", "unique"], mysql: ["normal", "unique", "fulltext", "spatial"] };
const INDEX_METHODS: IndexMethod[] = ["btree", "hash"];

const blankColumn = (): ColumnDesign => ({
  originalName: null,
  name: "",
  dataType: "VARCHAR(255)",
  nullable: true,
  default: null,
  autoIncrement: false,
  primaryKey: false,
  comment: null,
});

function newDesign(engine: Engine): TableDesign {
  const id: ColumnDesign = { ...blankColumn(), name: "id", dataType: engine === "sqlite" ? "INTEGER" : "INT", nullable: false, autoIncrement: true, primaryKey: true };
  return { name: "", columns: [id], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null }, primaryKeyOrder: ["id"] };
}

/** Names a new table, renaming foreign keys that still carry the name-derived default (`fk_<table>_N`). */
function withName(d: TableDesign, name: string): TableDesign {
  const auto = new RegExp(`^fk_${(d.name || "table").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}_(\\d+)$`);
  return {
    ...d,
    name,
    foreignKeys: d.foreignKeys.map((fk) => {
      const n = auto.exec(fk.name)?.[1];
      return n ? { ...fk, name: `fk_${name}_${n}` } : fk;
    }),
  };
}

const TIMESTAMP_COLUMNS = ["created_at", "updated_at"];

/** Appends `created_at` and `updated_at` DATETIME columns, skipping a name the design already has. */
function withTimestamps(d: TableDesign): TableDesign {
  const taken = new Set(d.columns.map((c) => c.name.toLowerCase()));
  const added = TIMESTAMP_COLUMNS.filter((name) => !taken.has(name)).map((name): ColumnDesign => ({ ...blankColumn(), name, dataType: "DATETIME" }));
  return { ...d, columns: [...d.columns, ...added] };
}

/** Message of a rejected `planTable` call (an `AppError` from the backend). */
const planError = (e: unknown) => (e && typeof e === "object" && "message" in e ? String(e.message) : String(e));

/**
 * Keyboard handler for a design table body: ArrowUp/ArrowDown move to the same field of the adjacent row, and
 * ArrowDown on the last row calls `append` (like Navicat), which declines by returning false; the new row opens on
 * its first field (the name). `append` gets the last row's element because fields that commit on blur may not have
 * reached the design yet.
 */
function rowArrowKeys(append: (lastRow: HTMLTableRowElement) => boolean) {
  return (e: KeyboardEvent<HTMLTableSectionElement>) => {
    if ((e.key !== "ArrowDown" && e.key !== "ArrowUp") || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || e.defaultPrevented) return;
    // Arrows change the value of a select.
    if (e.target instanceof HTMLSelectElement) return;
    const cell = (e.target as HTMLElement).closest("td");
    const row = cell?.parentElement;
    if (!cell || !(row instanceof HTMLTableRowElement)) return;
    const body = e.currentTarget;
    const fieldOf = (r: number, c?: number) => body.rows[r]?.querySelector<HTMLElement>(c === undefined ? "input, select" : `td:nth-child(${c + 1}) :is(input, select)`);
    const focusRow = (r: number) => fieldOf(r, cell.cellIndex)?.focus();
    e.preventDefault();
    const r = row.sectionRowIndex;
    if (e.key === "ArrowUp") focusRow(r - 1);
    else if (r < body.rows.length - 1) focusRow(r + 1);
    else if (append(row)) requestAnimationFrame(() => fieldOf(r + 1)?.focus());
  };
}

/** Whether the input labelled `label` in `row` holds text (read from the DOM: it may not be committed yet). */
const hasText = (row: HTMLTableRowElement, label: string) => !!row.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)?.value.trim();

const splitList = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);

type Props = {
  tabId: string;
  connectionId: string;
  database: string;
  engine: Engine;
  /** Existing table to alter; omitted for a new table. */
  table?: string;
  onSaved: (name: string) => void;
};

export function TableDesigner({ tabId, connectionId, database, engine, table, onSaved }: Props) {
  const qc = useQueryClient();
  const sqlite = engine === "sqlite";
  const original = useQuery({
    queryKey: ["tableDesign", connectionId, database, table],
    queryFn: () => api.tableDesign(connectionId, database, table!),
    enabled: !!table,
  });
  const draft = useDesignerDrafts((s) => s.drafts[tabId]);
  const design = draft?.design ?? null;
  const baseline = draft?.original ?? null;
  const setDesign = (change: TableDesign | ((d: TableDesign) => TableDesign)) =>
    useDesignerDrafts.getState().update(tabId, (d) => typeof change === "function" ? change(d) : change);
  const restructure = (change: (d: TableDesign) => TableDesign) => useDesignerDrafts.getState().restructure(tabId, change);
  const [section, setSection] = useState<Section>("columns");
  /** Selected column rows (by position) and the row Shift+click extends from. */
  const [picked, setPicked] = useState<{ rows: number[]; anchor: number | null }>({ rows: [], anchor: null });
  /** Open review dialog. Existing tables: the planned statements. New tables: the name being entered (planned below). */
  const [review, setReview] = useState<{ statements: string[] } | { name: string } | null>(null);
  const busy = draft?.busy ?? false;
  const setBusy = (value: boolean) => useDesignerDrafts.getState().setBusy(tabId, value);
  useEffect(() => {
    if (table) {
      if (original.data) useDesignerDrafts.getState().initialize(tabId, original.data, original.data);
    } else {
      useDesignerDrafts.getState().initialize(tabId, null, newDesign(engine));
    }
  }, [tabId, table, engine, original.data]);

  const objects = useQuery({ queryKey: ["objects", connectionId, database], queryFn: () => api.listObjects(connectionId, database) });
  const charsets = useQuery({ queryKey: ["charsets", connectionId], queryFn: () => api.listCharsets(connectionId), enabled: !sqlite, staleTime: Infinity });
  const designKey = JSON.stringify(design);
  const ddl = useQuery({
    queryKey: ["plan", connectionId, database, table, designKey],
    // A new table is named on save; preview it under a placeholder until then.
    queryFn: () => api.planTable(connectionId, database, baseline, table ? design! : withName(design!, design!.name || "new_table")),
    enabled: section === "ddl" && !!design && (!table || !!baseline),
    retry: false,
  });
  const reviewName = review && "name" in review ? review.name.trim() : "";
  const timestamps = useDesignerPrefs((s) => s.timestamps);
  /** The design a new table is created from: the entered name, plus the timestamp columns when chosen. */
  const finalDesign = (d: TableDesign) => (timestamps ? withTimestamps(withName(d, reviewName)) : withName(d, reviewName));
  const newPlan = useQuery({
    queryKey: ["plan", connectionId, database, null, designKey, reviewName, timestamps],
    queryFn: () => api.planTable(connectionId, database, null, finalDesign(design!)),
    enabled: !!design && reviewName !== "",
    retry: false,
    placeholderData: keepPreviousData,
  });
  const dirty = draft?.dirty ?? false;
  /** Latest Ctrl+S action, for the window-level shortcut (only the active tab's designer is mounted). */
  const saveShortcutRef = useRef<() => void>(() => {});
  /** Latest `startReview`, run after a blurred field's pending edit lands. */
  const startReviewRef = useRef<() => Promise<void>>(async () => {});
  const designerRef = useRef<HTMLDivElement>(null);
  /** Adds a row to the visible design table; used when ArrowDown/ArrowUp finds it empty. */
  const addRowRef = useRef<() => void>(() => {});
  useEffect(() => {
    const onKey = (e: WindowEventMap["keydown"]) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        // Navicat: with no field selected, an arrow key selects the last row; the next ArrowDown appends a row.
        if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || e.defaultPrevented) return;
        if (e.target instanceof Element && e.target.closest('input, select, textarea, [contenteditable="true"], tbody')) return;
        if (document.querySelector('[role="dialog"], [role="menu"], [role="listbox"]')) return;
        const body = designerRef.current?.querySelector<HTMLTableSectionElement>(".designer-body tbody");
        if (!body) return;
        e.preventDefault();
        const focusLast = () => body.rows[body.rows.length - 1]?.querySelector<HTMLElement>("input, select")?.focus();
        if (body.rows.length > 0) focusLast();
        else {
          addRowRef.current();
          requestAnimationFrame(focusLast);
        }
        return;
      }
      if (!isMod(e) || e.altKey || e.shiftKey || e.key.toLowerCase() !== "s") return;
      e.preventDefault();
      if (!e.repeat) saveShortcutRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  if (!design) return <div className="empty"><p>{original.isError ? (original.error as { message: string }).message : "Loading…"}</p></div>;

  const patch = (p: Partial<TableDesign>) => setDesign((d) => (d ? { ...d, ...p } : d));
  /** Changes column attributes other than its name and primary key flag (see `renameColumn`/`setPrimaryKey`). */
  const setColumn = (i: number, p: Partial<Omit<ColumnDesign, "name" | "primaryKey">>) =>
    setDesign((d) => (d ? { ...d, columns: d.columns.map((c, j) => (j === i ? { ...c, ...p } : c)) } : d));
  const selectedColumns = picked.rows.filter((i) => i < design.columns.length);
  /** Navicat-style row selection from the row header: click selects, Ctrl/⌘+click toggles, Shift+click extends. */
  const pickColumn = (i: number, e: MouseEvent) => {
    const anchor = picked.anchor !== null && picked.anchor < design.columns.length ? picked.anchor : null;
    if (e.shiftKey && anchor !== null) {
      const range = Array.from({ length: Math.abs(i - anchor) + 1 }, (_, k) => Math.min(i, anchor) + k);
      setPicked({ rows: [...new Set([...(isMod(e) ? selectedColumns : []), ...range])], anchor });
    } else if (isMod(e)) {
      setPicked({ rows: selectedColumns.includes(i) ? selectedColumns.filter((r) => r !== i) : [...selectedColumns, i], anchor: i });
    } else {
      setPicked({ rows: [i], anchor: i });
    }
  };
  const moveSelected = (delta: -1 | 1) => {
    let next = picked;
    restructure((d) => {
      const result = moveColumns(d, selectedColumns, delta);
      // The anchor follows its column, whether or not it is still selected.
      next = { rows: result.selected, anchor: picked.anchor === null ? null : result.moved[picked.anchor] ?? null };
      return result.design;
    });
    setPicked(next);
  };
  /** Revert replaces the columns, so positions no longer name the selected columns. */
  const revert = () => {
    useDesignerDrafts.getState().revert(tabId);
    setPicked({ rows: [], anchor: null });
  };
  const deleteSelected = () => {
    restructure((d) => removeColumns(d, selectedColumns));
    setPicked({ rows: [], anchor: null });
  };
  const setIndex = (i: number, p: Partial<IndexDesign>) => patch({ indexes: design.indexes.map((x, j) => (j === i ? { ...x, ...p } : x)) });
  const setFk = (i: number, p: Partial<ForeignKeyDesign>) => patch({ foreignKeys: design.foreignKeys.map((x, j) => (j === i ? { ...x, ...p } : x)) });
  const addColumn = () => setDesign((d) => (d ? { ...d, columns: [...d.columns, blankColumn()] } : d));
  const addIndex = () => setDesign((d) => (d ? { ...d, indexes: [...d.indexes, { name: "", columns: [], kind: "normal", method: null, comment: null }] } : d));
  const addFk = () =>
    setDesign((d) =>
      d ? { ...d, foreignKeys: [...d.foreignKeys, { name: `fk_${d.name || "table"}_${d.foreignKeys.length + 1}`, columns: [], refTable: "", refSchema: null, refColumns: [], onUpdate: "NO ACTION", onDelete: "NO ACTION" }] } : d,
    );
  addRowRef.current = section === "indexes" ? addIndex : section === "foreignKeys" ? addFk : addColumn;

  const startReview = async () => {
    const store = useDesignerDrafts.getState();
    if (store.drafts[tabId]?.busy) return;
    store.flush(tabId);
    const current = useDesignerDrafts.getState().drafts[tabId];
    if (!current) return;
    setBusy(true);
    try {
      if (table) {
        const statements = await api.planTable(connectionId, database, current.original, current.design);
        if (statements.length === 0) notify("No changes to save");
        else setReview({ statements });
      } else {
        // Surface design errors before asking for the name.
        await api.planTable(connectionId, database, null, withName(current.design, current.design.name || "new_table"));
        setReview({ name: current.design.name });
      }
    } catch (e) {
      notifyError("The design is not valid", e);
    } finally {
      setBusy(false);
    }
  };
  startReviewRef.current = startReview;
  const statements = !review ? null : "statements" in review ? review.statements : reviewName ? newPlan.data ?? null : null;
  const canApply = !busy && !!statements && (!!table || (reviewName !== "" && !newPlan.isFetching && !newPlan.isError));
  const apply = async () => {
    if (!canApply || !statements || useDesignerDrafts.getState().drafts[tabId]?.busy) return;
    const saved = table ? design : finalDesign(design);
    setBusy(true);
    try {
      if (useGridStates.getState().states[tabId]?.committing) {
        notify("Save in progress", "Wait for the pending grid save to finish before altering this table.");
        return;
      }
      // Pending grid edits address rows of the table as it is now; altering it under them could write them to
      // the wrong columns or rows, so they are discarded first.
      if (table && hasPendingEdits(tabId)) {
        if (!(await confirmAction({
          title: "Discard grid changes",
          message: "This table has unsaved grid edits. Altering it discards them. Continue?",
          confirmLabel: "Discard and alter",
          danger: true,
        }))) return;
        if (!useGridStates.getState().discard(tabId)) {
          notify("Save in progress", "Wait for the pending grid save to finish before altering this table.");
          return;
        }
      }
      const reviewed = useDesignerDrafts.getState().drafts[tabId];
      await api.applyTableDesign(connectionId, database, table ? reviewed?.original ?? null : null, saved, statements);
      setReview(null);
      const canonical = await api.tableDesign(connectionId, database, saved.name).catch((error) => {
        notifyError("Table saved, but its design could not be reloaded", error);
        return { ...saved, columns: saved.columns.map((column) => ({ ...column, originalName: column.name })) };
      });
      useDesignerDrafts.getState().saved(tabId, canonical);
      qc.setQueryData(["tableDesign", connectionId, database, saved.name], canonical);
      notify(table ? "Table altered" : "Table created", saved.name);
      await qc.invalidateQueries({ queryKey: ["objects", connectionId, database] });
      await qc.invalidateQueries({ queryKey: ["tableInfo", connectionId, database] });
      await qc.invalidateQueries({ queryKey: ["page", connectionId, database] });
      onSaved(saved.name);
    } catch (e) {
      notifyError(table ? "The table was not altered" : "The table was not created", e);
    } finally {
      setBusy(false);
    }
  };
  // Same as the Save button: review the SQL first; in the review, Ctrl+S applies it.
  saveShortcutRef.current = () => {
    if (review) {
      void apply();
      return;
    }
    if (busy || !dirty) return;
    // Blur first so a field that commits on blur (column name, index columns) lands in the design.
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    requestAnimationFrame(() => void startReviewRef.current());
  };

  const columnNames = design.columns.map((c) => c.name).filter(Boolean);
  const typeOptions = typeSuggestions(engine);
  const engines = engine === "mariaDb" ? MARIADB_ENGINES : MYSQL_ENGINES;
  const currentCharset = charsets.data?.find((c) => c.name === design.options.charset);
  const sections: { id: Section; label: string; count?: number }[] = [
    { id: "columns", label: "Columns", count: design.columns.length },
    { id: "indexes", label: "Indexes", count: design.indexes.length },
    { id: "foreignKeys", label: "Foreign Keys", count: design.foreignKeys.length },
    ...(sqlite ? [] : [{ id: "options" as const, label: "Options" }]),
    { id: "ddl", label: "SQL Preview" },
  ];

  return (
    <div className="designer" ref={designerRef}>
      <div className="action-bar">
        {table && (
          <label className="inline-field">
            <span>Table name</span>
            <input className="input" value={design.name} onChange={(e) => patch({ name: e.target.value })} aria-label="Table name" />
          </label>
        )}
        {section === "columns" && (
          <>
            {table && <div className="toolbar-sep" />}
            <button className="obj-btn" disabled={busy || !canMoveColumns(selectedColumns, design.columns.length, -1)} onClick={() => moveSelected(-1)}><ArrowUp size={15} />Move Up</button>
            <button className="obj-btn" disabled={busy || !canMoveColumns(selectedColumns, design.columns.length, 1)} onClick={() => moveSelected(1)}><ArrowDown size={15} />Move Down</button>
            <button className="obj-btn" disabled={busy || selectedColumns.length === 0} onClick={deleteSelected}><Trash2 size={15} className="ob-delete" />{selectedColumns.length > 1 ? `Delete ${selectedColumns.length} Columns` : "Delete Column"}</button>
          </>
        )}
        <div className="grow" />
        {table && <button className="btn" disabled={!dirty || busy} onClick={revert}><Undo2 size={15} />Revert</button>}
        <button className="btn primary" disabled={busy || !dirty || (!!table && !design.name.trim())} onClick={() => void startReview()} title={`${MOD}+S`}><Save size={15} />{table ? "Save Changes" : "Create Table"}</button>
      </div>
      <div className="sheet-tabs" role="tablist">
        {sections.map((s) => (
          <button key={s.id} role="tab" aria-selected={section === s.id} className={section === s.id ? "on" : ""} onClick={() => setSection(s.id)}>
            {s.label}{s.count !== undefined && <span className="sheet-tab-count">{s.count}</span>}
          </button>
        ))}
      </div>

      <div className="table-scroll designer-body" inert={busy}>
        {section === "columns" && (
          <>
            <table className="list-table design-table">
              <thead>
                <tr>
                  <th style={{ width: 28 }} />
                  <th>Name</th><th>Type</th><th>Length / Values</th>
                  {!sqlite && <th title="Unsigned">Uns.</th>}
                  <th>Not Null</th><th>Default</th><th title="Auto increment">Auto Inc.</th><th title="Primary key">PK</th>
                  {!sqlite && <th>Comment</th>}
                </tr>
              </thead>
              <tbody onKeyDown={rowArrowKeys((row) => { if (!hasText(row, "Column name")) return false; addColumn(); return true; })}>
                {design.columns.map((c, i) => {
                  const t = parseType(c.dataType);
                  const setType = (p: Partial<typeof t>) => setColumn(i, { dataType: composeType({ ...t, ...p }) });
                  const loaded = c.originalName === null ? null : baseline?.columns.find((o) => o.name === c.originalName)?.dataType ?? null;
                  return (
                    <tr
                      key={i}
                      className={selectedColumns.includes(i) ? "selected" : undefined}
                      aria-selected={selectedColumns.includes(i)}
                      // Editing a field selects its row, unless it is already part of the selection.
                      onFocus={() => !selectedColumns.includes(i) && setPicked({ rows: [i], anchor: i })}
                    >
                      <td
                        className="center row-header"
                        title={`Select column (${MOD}+click or Shift+click to select several)`}
                        onMouseDown={(e) => {
                          // No text selection on Shift+click; the edited field commits on blur.
                          e.preventDefault();
                          if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
                          pickColumn(i, e);
                        }}
                      >
                        {c.primaryKey && <KeyRound size={14} className="pk-icon" />}
                      </td>
                      <td><CommitInput tabId={tabId} field={`column:${i}`} value={c.name} label="Column name" onCommit={(name) => setDesign((d) => renameColumn(d, i, name))} /></td>
                      <td><ComboInput value={t.base} options={typeOptions} label="Column type" onChange={(base) => setColumn(i, { dataType: retype(engine, c.dataType, loaded, base) })} /></td>
                      <td><input className="cell-input narrow" value={t.length} aria-label="Length" onChange={(e) => setType({ length: e.target.value })} /></td>
                      {!sqlite && <td className="center"><input type="checkbox" disabled={!isNumericType(t.base)} checked={t.unsigned} aria-label="Unsigned" onChange={(e) => setType({ unsigned: e.target.checked })} /></td>}
                      <td className="center"><input type="checkbox" checked={!c.nullable} aria-label="Not null" onChange={(e) => setColumn(i, { nullable: !e.target.checked })} /></td>
                      <td><input className="cell-input" value={c.default ?? ""} placeholder="NULL" aria-label="Default" onChange={(e) => setColumn(i, { default: e.target.value === "" ? null : e.target.value })} /></td>
                      <td className="center"><input type="checkbox" checked={c.autoIncrement} aria-label="Auto increment" onChange={(e) => setColumn(i, { autoIncrement: e.target.checked })} /></td>
                      <td className="center"><input type="checkbox" checked={c.primaryKey} aria-label="Primary key" onChange={(e) => setDesign((d) => setPrimaryKey(d, i, e.target.checked))} /></td>
                      {!sqlite && <td><input className="cell-input" value={c.comment ?? ""} aria-label="Comment" onChange={(e) => setColumn(i, { comment: e.target.value || null })} /></td>}
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <div className="designer-add"><button className="btn sm" onClick={addColumn}><Plus size={13} />Add Column</button></div>
            <p className="hint designer-hint">Defaults are SQL expressions: quote text ('pending'), use numbers (0) or functions (CURRENT_TIMESTAMP).</p>
          </>
        )}

        {section === "indexes" && (
          <>
            <table className="list-table design-table">
              <thead>
                <tr>
                  <th>Name</th><th>Columns (comma separated)</th><th>Index Type</th>
                  {!sqlite && <><th>Index Method</th><th>Comment</th></>}
                  <th style={{ width: 40 }} />
                </tr>
              </thead>
              <tbody onKeyDown={rowArrowKeys((row) => { if (!hasText(row, "Columns")) return false; addIndex(); return true; })}>
                {design.indexes.map((x, i) => (
                  <tr key={i}>
                    <td><input className="cell-input" value={x.name} placeholder={x.columns.length ? `idx_${x.columns.join("_")}` : "idx_<columns>"} aria-label="Index name" onChange={(e) => setIndex(i, { name: e.target.value })} /></td>
                    <td><ColumnList tabId={tabId} field={`index:${i}`} value={x.columns} options={columnNames} onChange={(columns) => setIndex(i, { columns })} /></td>
                    <td>
                      <select
                        className="cell-input"
                        value={x.kind}
                        aria-label="Index type"
                        // FULLTEXT and SPATIAL indexes have a fixed structure, so they take no method.
                        onChange={(e) => { const kind = e.target.value as IndexKind; setIndex(i, { kind, method: kind === "normal" || kind === "unique" ? x.method : null }); }}
                      >
                        {INDEX_KINDS[sqlite ? "sqlite" : "mysql"].map((k) => <option key={k} value={k}>{k.toUpperCase()}</option>)}
                      </select>
                    </td>
                    {!sqlite && (
                      <>
                        <td>
                          <select
                            className="cell-input"
                            value={x.method ?? ""}
                            aria-label="Index method"
                            disabled={x.kind === "fulltext" || x.kind === "spatial"}
                            onChange={(e) => setIndex(i, { method: (e.target.value || null) as IndexMethod | null })}
                          >
                            <option value="" />
                            {INDEX_METHODS.map((m) => <option key={m} value={m}>{m.toUpperCase()}</option>)}
                          </select>
                        </td>
                        <td><input className="cell-input" value={x.comment ?? ""} aria-label="Index comment" onChange={(e) => setIndex(i, { comment: e.target.value || null })} /></td>
                      </>
                    )}
                    <td><button className="icon-btn danger" title="Delete index" onClick={() => restructure((d) => ({ ...d, indexes: d.indexes.filter((_, j) => j !== i) }))}><Trash2 size={14} /></button></td>
                  </tr>
                ))}
                {design.indexes.length === 0 && <tr><td colSpan={sqlite ? 4 : 6} className="muted">No indexes</td></tr>}
              </tbody>
            </table>
            <div className="designer-add">
              {/* An empty name becomes idx_<columns> and an empty method BTREE when the design is applied. */}
              <button className="btn sm" onClick={addIndex}><Plus size={13} />Add Index</button>
            </div>
          </>
        )}

        {section === "foreignKeys" && (
          <>
            <table className="list-table design-table">
              <thead><tr><th>Name</th><th>Columns</th><th>Referenced table</th><th>Referenced columns</th><th>On Update</th><th>On Delete</th><th style={{ width: 40 }} /></tr></thead>
              <tbody onKeyDown={rowArrowKeys((row) => { if (!hasText(row, "Columns")) return false; addFk(); return true; })}>
                {design.foreignKeys.map((f, i) => (
                  <tr key={i}>
                    <td><input className="cell-input" value={f.name} aria-label="Foreign key name" onChange={(e) => setFk(i, { name: e.target.value })} /></td>
                    <td><ColumnList tabId={tabId} field={`fk:${i}:columns`} value={f.columns} options={columnNames} onChange={(columns) => setFk(i, { columns })} /></td>
                    <td>
                      <select className="cell-input" value={f.refTable} aria-label="Referenced table" onChange={(e) => setFk(i, { refTable: e.target.value })}>
                        <option value="" />
                        {objects.data?.filter((o) => o.kind === "table").map((o) => <option key={o.name} value={o.name}>{o.name}</option>)}
                      </select>
                    </td>
                    <td><ColumnList tabId={tabId} field={`fk:${i}:references`} value={f.refColumns} onChange={(refColumns) => setFk(i, { refColumns })} /></td>
                    <td><select className="cell-input" value={f.onUpdate} aria-label="On update" onChange={(e) => setFk(i, { onUpdate: e.target.value })}>{FK_ACTIONS.map((a) => <option key={a}>{a}</option>)}</select></td>
                    <td><select className="cell-input" value={f.onDelete} aria-label="On delete" onChange={(e) => setFk(i, { onDelete: e.target.value })}>{FK_ACTIONS.map((a) => <option key={a}>{a}</option>)}</select></td>
                    <td><button className="icon-btn danger" title="Delete foreign key" onClick={() => restructure((d) => ({ ...d, foreignKeys: d.foreignKeys.filter((_, j) => j !== i) }))}><Trash2 size={14} /></button></td>
                  </tr>
                ))}
                {design.foreignKeys.length === 0 && <tr><td colSpan={7} className="muted">No foreign keys</td></tr>}
              </tbody>
            </table>
            <div className="designer-add">
              <button className="btn sm" onClick={addFk}>
                <Plus size={13} />Add Foreign Key
              </button>
            </div>
          </>
        )}

        {section === "options" && (
          <div className="form designer-options">
            <label className="field"><span>Engine</span>
              <select value={design.options.engine ?? ""} onChange={(e) => patch({ options: { ...design.options, engine: e.target.value || null } })}>
                <option value="">Server default</option>
                {engines.map((x) => <option key={x}>{x}</option>)}
              </select>
            </label>
            <div className="row">
              <label className="field grow"><span>Character set</span>
                <select value={design.options.charset ?? ""} onChange={(e) => patch({ options: { ...design.options, charset: e.target.value || null, collation: null } })}>
                  <option value="">Database default</option>
                  {charsets.data?.map((c) => <option key={c.name}>{c.name}</option>)}
                </select>
              </label>
              <label className="field grow"><span>Collation</span>
                <select value={design.options.collation ?? ""} onChange={(e) => patch({ options: { ...design.options, collation: e.target.value || null } })}>
                  <option value="">Default</option>
                  {currentCharset?.collations.map((c) => <option key={c}>{c}</option>)}
                </select>
              </label>
            </div>
            <label className="field"><span>Comment</span>
              <input value={design.options.comment ?? ""} onChange={(e) => patch({ options: { ...design.options, comment: e.target.value || null } })} />
            </label>
          </div>
        )}

        {section === "ddl" && (
          <div className="ddl-preview">
            {ddl.isError ? (
              <p className="error-text">{(ddl.error as { message: string }).message}</p>
            ) : ddl.data && ddl.data.length === 0 ? (
              <p className="muted">No changes.</p>
            ) : (
              <pre className="code-block">{ddl.data ? ddl.data.map((s) => `${s};`).join("\n\n") : "…"}</pre>
            )}
          </div>
        )}
      </div>

      {review && (
        <Dialog
          open
          onClose={() => setReview(null)}
          title={table ? "Review changes" : "Create table"}
          width={720}
          footer={
            <>
              <button className="btn" disabled={!statements} onClick={() => statements && void navigator.clipboard.writeText(statements.map((s) => `${s};`).join("\n\n")).then(() => notify("SQL copied"))}><Copy size={14} />Copy SQL</button>
              <div className="grow" />
              <button className="btn" onClick={() => setReview(null)}>Cancel</button>
              <button className="btn primary" disabled={!canApply} onClick={() => void apply()}>{table ? "Apply" : "Create Table"}</button>
            </>
          }
        >
          {"name" in review && (
            <label className="field review-name">
              <span>Table name</span>
              <input
                className="input"
                value={review.name}
                onChange={(e) => setReview({ name: e.target.value })}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  e.preventDefault();
                  void apply();
                }}
                placeholder="new_table"
                autoFocus
                spellCheck={false}
                aria-label="Table name"
              />
            </label>
          )}
          {"name" in review && (
            <label className="check review-timestamps">
              <input type="checkbox" checked={timestamps} onChange={(e) => useDesignerPrefs.getState().setTimestamps(e.target.checked)} />
              Add <code>created_at</code> and <code>updated_at</code> (DATETIME)
            </label>
          )}
          <p className="dialog-text">The following SQL will run{statements && statements.length > 1 ? " in order" : ""}:</p>
          {"name" in review && newPlan.isError ? (
            <p className="error-text">{planError(newPlan.error)}</p>
          ) : (
            <pre className="code-block tall">{statements ? statements.map((s) => `${s};`).join("\n\n") : reviewName ? "…" : "Enter a table name."}</pre>
          )}
        </Dialog>
      )}
    </div>
  );
}

function ColumnList({ tabId, field, value, options, onChange }: { tabId: string; field: string; value: string[]; options?: string[]; onChange: (v: string[]) => void }) {
  const text = useDesignerDrafts((s) => s.drafts[tabId]?.inputs[field]) ?? value.join(", ");
  const input = useDesignerDrafts((s) => s.input);
  const id = useMemo(() => `cols-${Math.random().toString(36).slice(2)}`, []);
  return (
    <>
      <input className="cell-input" list={options ? id : undefined} value={text} aria-label="Columns" onChange={(e) => input(tabId, field, e.target.value === value.join(", ") ? null : e.target.value)} onBlur={() => { onChange(splitList(text)); input(tabId, field, null); }} />
      {options && <datalist id={id}>{options.map((o) => <option key={o} value={o} />)}</datalist>}
    </>
  );
}

/** Text input that reports its value on blur or Enter, so renames propagate once per edit. */
function CommitInput({ tabId, field, value, label, onCommit }: { tabId: string; field: string; value: string; label: string; onCommit: (v: string) => void }) {
  const text = useDesignerDrafts((s) => s.drafts[tabId]?.inputs[field]) ?? value;
  const input = useDesignerDrafts((s) => s.input);
  const commit = () => {
    if (text !== value) onCommit(text.trim());
    input(tabId, field, null);
  };
  return (
    <input
      className="cell-input"
      value={text}
      aria-label={label}
      onChange={(e) => input(tabId, field, e.target.value === value ? null : e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
    />
  );
}
