import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Copy, KeyRound, Plus, Save, Trash2, Undo2 } from "lucide-react";
import { api } from "../api/client";
import type { ColumnDesign, Engine, ForeignKeyDesign, IndexDesign, IndexKind, IndexMethod, TableDesign } from "../api/types";
import { FK_ACTIONS, MARIADB_ENGINES, MYSQL_ENGINES, composeType, isNumericType, lengthForType, parseType, typeSuggestions } from "../lib/columnTypes";
import { notify, notifyError } from "../store/toasts";
import { isMod } from "../lib/platform";
import { Dialog } from "./Dialog";

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
  return { name: "", columns: [id], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null } };
}

const splitList = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);

type Props = {
  connectionId: string;
  database: string;
  engine: Engine;
  /** Existing table to alter; omitted for a new table. */
  table?: string;
  onSaved: (name: string) => void;
};

export function TableDesigner({ connectionId, database, engine, table, onSaved }: Props) {
  const qc = useQueryClient();
  const sqlite = engine === "sqlite";
  const original = useQuery({
    queryKey: ["tableDesign", connectionId, database, table],
    queryFn: () => api.tableDesign(connectionId, database, table!),
    enabled: !!table,
  });
  const [design, setDesign] = useState<TableDesign | null>(table ? null : newDesign(engine));
  const [section, setSection] = useState<Section>("columns");
  const [preview, setPreview] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (original.data) setDesign(structuredClone(original.data));
  }, [original.data]);

  const objects = useQuery({ queryKey: ["objects", connectionId, database], queryFn: () => api.listObjects(connectionId, database) });
  const charsets = useQuery({ queryKey: ["charsets", connectionId], queryFn: () => api.listCharsets(connectionId), enabled: !sqlite, staleTime: Infinity });
  const designKey = JSON.stringify(design);
  const ddl = useQuery({
    queryKey: ["plan", connectionId, database, table, designKey],
    queryFn: () => api.planTable(connectionId, database, original.data ?? null, design!),
    enabled: section === "ddl" && !!design && (!table || !!original.data),
    retry: false,
  });
  const dirty = useMemo(() => (table ? JSON.stringify(original.data) !== designKey : true), [table, original.data, designKey]);
  /** Latest `review`, for the Ctrl+S handler that runs it after pending field edits land. */
  const reviewRef = useRef<() => Promise<void>>(async () => {});

  if (!design) return <div className="empty"><p>{original.isError ? (original.error as { message: string }).message : "Loading…"}</p></div>;

  const patch = (p: Partial<TableDesign>) => setDesign((d) => (d ? { ...d, ...p } : d));
  const setColumn = (i: number, p: Partial<ColumnDesign>) =>
    setDesign((d) => {
      if (!d) return d;
      const old = d.columns[i].name;
      const columns = d.columns.map((c, j) => (j === i ? { ...c, ...p } : c));
      // Keep index and foreign key references in step with renamed columns.
      const rename = (cols: string[]) => (p.name !== undefined && old && p.name ? cols.map((c) => (c === old ? p.name! : c)) : cols);
      return {
        ...d,
        columns,
        indexes: d.indexes.map((x) => ({ ...x, columns: rename(x.columns) })),
        foreignKeys: d.foreignKeys.map((x) => ({ ...x, columns: rename(x.columns) })),
      };
    });
  const moveColumn = (i: number, delta: number) =>
    setDesign((d) => {
      if (!d) return d;
      const j = i + delta;
      if (j < 0 || j >= d.columns.length) return d;
      const columns = [...d.columns];
      [columns[i], columns[j]] = [columns[j], columns[i]];
      return { ...d, columns };
    });
  const setIndex = (i: number, p: Partial<IndexDesign>) => patch({ indexes: design.indexes.map((x, j) => (j === i ? { ...x, ...p } : x)) });
  const setFk = (i: number, p: Partial<ForeignKeyDesign>) => patch({ foreignKeys: design.foreignKeys.map((x, j) => (j === i ? { ...x, ...p } : x)) });

  const review = async () => {
    setBusy(true);
    try {
      const statements = await api.planTable(connectionId, database, original.data ?? null, design);
      if (statements.length === 0) notify("No changes to save");
      else setPreview(statements);
    } catch (e) {
      notifyError("The design is not valid", e);
    } finally {
      setBusy(false);
    }
  };
  const apply = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      await api.applyDdl(connectionId, database, preview);
      setPreview(null);
      notify(table ? "Table altered" : "Table created", design.name);
      await qc.invalidateQueries({ queryKey: ["objects", connectionId, database] });
      await qc.invalidateQueries({ queryKey: ["tableInfo", connectionId, database] });
      await qc.invalidateQueries({ queryKey: ["tableDesign", connectionId, database] });
      await qc.invalidateQueries({ queryKey: ["page", connectionId, database] });
      onSaved(design.name);
    } catch (e) {
      notifyError(table ? "The table was not altered" : "The table was not created", e);
    } finally {
      setBusy(false);
    }
  };
  reviewRef.current = review;

  const columnNames = design.columns.map((c) => c.name).filter(Boolean);
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
    <div
      className="designer"
      onKeyDown={(e) => {
        if (!isMod(e) || e.key.toLowerCase() !== "s") return;
        e.preventDefault();
        // Same as the Save button: review the SQL first; in the review, Ctrl+S applies it.
        if (preview) {
          if (!busy) void apply();
          return;
        }
        // Blur first so a field that commits on blur (column name, index columns) lands in the design.
        if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
        requestAnimationFrame(() => reviewRef.current());
      }}
    >
      <div className="action-bar">
        <label className="inline-field">
          <span>Table name</span>
          <input className="input" value={design.name} onChange={(e) => patch({ name: e.target.value })} placeholder="new_table" autoFocus={!table} aria-label="Table name" />
        </label>
        <div className="segmented">
          {sections.map((s) => (
            <button key={s.id} className={section === s.id ? "on" : ""} onClick={() => setSection(s.id)}>
              {s.label}{s.count !== undefined && <span className="seg-count">{s.count}</span>}
            </button>
          ))}
        </div>
        <div className="grow" />
        {table && <button className="btn" disabled={!dirty || busy} onClick={() => original.data && setDesign(structuredClone(original.data))}><Undo2 size={15} />Revert</button>}
        <button className="btn primary" disabled={busy || !dirty || !design.name.trim()} onClick={review}><Save size={15} />{table ? "Save Changes" : "Create Table"}</button>
      </div>

      <div className="table-scroll designer-body">
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
                  <th style={{ width: 96 }} />
                </tr>
              </thead>
              <tbody>
                <datalist id="type-suggestions">{typeSuggestions(engine).map((t) => <option key={t} value={t} />)}</datalist>
                {design.columns.map((c, i) => {
                  const t = parseType(c.dataType);
                  const setType = (p: Partial<typeof t>) => {
                    const n = { ...t, ...p };
                    setColumn(i, { dataType: composeType(n.base, n.length, n.unsigned) });
                  };
                  return (
                    <tr key={i}>
                      <td className="center">{c.primaryKey && <KeyRound size={14} className="pk-icon" />}</td>
                      <td><CommitInput value={c.name} label="Column name" onCommit={(name) => setColumn(i, { name })} /></td>
                      <td><input className="cell-input" list="type-suggestions" value={t.base} aria-label="Column type" onChange={(e) => {
                            const base = e.target.value;
                            // Like Navicat, a new type brings its usual length (INT → 11, DECIMAL → 10,2).
                            setType({ base, length: lengthForType(engine, base, t.length), unsigned: t.unsigned && isNumericType(base) });
                          }} /></td>
                      <td><input className="cell-input narrow" value={t.length} aria-label="Length" onChange={(e) => setType({ length: e.target.value })} /></td>
                      {!sqlite && <td className="center"><input type="checkbox" disabled={!isNumericType(t.base)} checked={t.unsigned} aria-label="Unsigned" onChange={(e) => setType({ unsigned: e.target.checked })} /></td>}
                      <td className="center"><input type="checkbox" checked={!c.nullable} aria-label="Not null" onChange={(e) => setColumn(i, { nullable: !e.target.checked })} /></td>
                      <td><input className="cell-input" value={c.default ?? ""} placeholder="NULL" aria-label="Default" onChange={(e) => setColumn(i, { default: e.target.value === "" ? null : e.target.value })} /></td>
                      <td className="center"><input type="checkbox" checked={c.autoIncrement} aria-label="Auto increment" onChange={(e) => setColumn(i, { autoIncrement: e.target.checked })} /></td>
                      <td className="center"><input type="checkbox" checked={c.primaryKey} aria-label="Primary key" onChange={(e) => setColumn(i, { primaryKey: e.target.checked, nullable: e.target.checked ? false : c.nullable })} /></td>
                      {!sqlite && <td><input className="cell-input" value={c.comment ?? ""} aria-label="Comment" onChange={(e) => setColumn(i, { comment: e.target.value || null })} /></td>}
                      <td className="row-actions">
                        <button className="icon-btn" title="Move up" disabled={i === 0} onClick={() => moveColumn(i, -1)}><ArrowUp size={14} /></button>
                        <button className="icon-btn" title="Move down" disabled={i === design.columns.length - 1} onClick={() => moveColumn(i, 1)}><ArrowDown size={14} /></button>
                        <button className="icon-btn danger" title="Delete column" onClick={() => patch({ columns: design.columns.filter((_, j) => j !== i) })}><Trash2 size={14} /></button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <div className="designer-add"><button className="btn sm" onClick={() => patch({ columns: [...design.columns, blankColumn()] })}><Plus size={13} />Add Column</button></div>
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
              <tbody>
                {design.indexes.map((x, i) => (
                  <tr key={i}>
                    <td><input className="cell-input" value={x.name} aria-label="Index name" onChange={(e) => setIndex(i, { name: e.target.value })} /></td>
                    <td><ColumnList value={x.columns} options={columnNames} onChange={(columns) => setIndex(i, { columns })} /></td>
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
                    <td><button className="icon-btn danger" title="Delete index" onClick={() => patch({ indexes: design.indexes.filter((_, j) => j !== i) })}><Trash2 size={14} /></button></td>
                  </tr>
                ))}
                {design.indexes.length === 0 && <tr><td colSpan={sqlite ? 4 : 6} className="muted">No indexes</td></tr>}
              </tbody>
            </table>
            <div className="designer-add">
              <button className="btn sm" onClick={() => patch({ indexes: [...design.indexes, { name: `ix_${design.name || "table"}_${design.indexes.length + 1}`, columns: [], kind: "normal", method: null, comment: null }] })}><Plus size={13} />Add Index</button>
            </div>
          </>
        )}

        {section === "foreignKeys" && (
          <>
            <table className="list-table design-table">
              <thead><tr><th>Name</th><th>Columns</th><th>Referenced table</th><th>Referenced columns</th><th>On Update</th><th>On Delete</th><th style={{ width: 40 }} /></tr></thead>
              <tbody>
                {design.foreignKeys.map((f, i) => (
                  <tr key={i}>
                    <td><input className="cell-input" value={f.name} aria-label="Foreign key name" onChange={(e) => setFk(i, { name: e.target.value })} /></td>
                    <td><ColumnList value={f.columns} options={columnNames} onChange={(columns) => setFk(i, { columns })} /></td>
                    <td>
                      <select className="cell-input" value={f.refTable} aria-label="Referenced table" onChange={(e) => setFk(i, { refTable: e.target.value })}>
                        <option value="" />
                        {objects.data?.filter((o) => o.kind === "table").map((o) => <option key={o.name} value={o.name}>{o.name}</option>)}
                      </select>
                    </td>
                    <td><ColumnList value={f.refColumns} onChange={(refColumns) => setFk(i, { refColumns })} /></td>
                    <td><select className="cell-input" value={f.onUpdate} aria-label="On update" onChange={(e) => setFk(i, { onUpdate: e.target.value })}>{FK_ACTIONS.map((a) => <option key={a}>{a}</option>)}</select></td>
                    <td><select className="cell-input" value={f.onDelete} aria-label="On delete" onChange={(e) => setFk(i, { onDelete: e.target.value })}>{FK_ACTIONS.map((a) => <option key={a}>{a}</option>)}</select></td>
                    <td><button className="icon-btn danger" title="Delete foreign key" onClick={() => patch({ foreignKeys: design.foreignKeys.filter((_, j) => j !== i) })}><Trash2 size={14} /></button></td>
                  </tr>
                ))}
                {design.foreignKeys.length === 0 && <tr><td colSpan={7} className="muted">No foreign keys</td></tr>}
              </tbody>
            </table>
            <div className="designer-add">
              <button className="btn sm" onClick={() => patch({ foreignKeys: [...design.foreignKeys, { name: `fk_${design.name || "table"}_${design.foreignKeys.length + 1}`, columns: [], refTable: "", refColumns: [], onUpdate: "NO ACTION", onDelete: "NO ACTION" }] })}>
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

      {preview && (
        <Dialog
          open
          onClose={() => setPreview(null)}
          title={table ? "Review changes" : "Review new table"}
          width={720}
          footer={
            <>
              <button className="btn" onClick={() => void navigator.clipboard.writeText(preview.map((s) => `${s};`).join("\n\n")).then(() => notify("SQL copied"))}><Copy size={14} />Copy SQL</button>
              <div className="grow" />
              <button className="btn" onClick={() => setPreview(null)}>Cancel</button>
              <button className="btn primary" disabled={busy} onClick={apply}>Apply</button>
            </>
          }
        >
          <p className="dialog-text">The following SQL will run{preview.length > 1 ? " in order" : ""}:</p>
          <pre className="code-block tall">{preview.map((s) => `${s};`).join("\n\n")}</pre>
        </Dialog>
      )}
    </div>
  );
}

function ColumnList({ value, options, onChange }: { value: string[]; options?: string[]; onChange: (v: string[]) => void }) {
  const [text, setText] = useState(value.join(", "));
  useEffect(() => setText(value.join(", ")), [value]);
  const id = useMemo(() => `cols-${Math.random().toString(36).slice(2)}`, []);
  return (
    <>
      <input className="cell-input" list={options ? id : undefined} value={text} aria-label="Columns" onChange={(e) => setText(e.target.value)} onBlur={() => onChange(splitList(text))} />
      {options && <datalist id={id}>{options.map((o) => <option key={o} value={o} />)}</datalist>}
    </>
  );
}

/** Text input that reports its value on blur or Enter, so renames propagate once per edit. */
function CommitInput({ value, label, onCommit }: { value: string; label: string; onCommit: (v: string) => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const commit = () => text !== value && onCommit(text.trim());
  return (
    <input
      className="cell-input"
      value={text}
      aria-label={label}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
    />
  );
}
