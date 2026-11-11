import { useQuery } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { api } from "../api/client";
import type { TableTab } from "../store/tabs";
import { DataGrid } from "./DataGrid";
import { formatCount, orDash } from "../lib/format";
import { notify } from "../store/toasts";

export function TableTabView({ tab }: { tab: TableTab }) {
  switch (tab.view) {
    case "data":
      return <DataGrid tab={tab} />;
    case "structure":
      return <StructureView tab={tab} />;
    case "info":
      return <InfoView tab={tab} />;
    default: {
      const never: never = tab.view;
      return never;
    }
  }
}

export function useTableInfo(tab: TableTab) {
  return useQuery({
    queryKey: ["tableInfo", tab.connectionId, tab.database, tab.name],
    queryFn: () => api.tableInfo(tab.connectionId, tab.database, tab.name),
  });
}

function StructureView({ tab }: { tab: TableTab }) {
  const info = useTableInfo(tab);
  const t = info.data;
  if (!t) return <div className="empty"><p>{info.isError ? (info.error as { message: string }).message : "Loading…"}</p></div>;
  return (
    <div className="table-scroll structure">
      <h4 className="block-title">Columns</h4>
      <table className="list-table">
        <thead><tr><th>Name</th><th>Type</th><th>Nullable</th><th>Default</th><th>Key</th><th>Extra</th><th>Comment</th></tr></thead>
        <tbody>
          {t.columns.map((c) => (
            <tr key={c.name}>
              <td><b>{c.name}</b></td>
              <td className="mono">{c.dataType}</td>
              <td>{c.nullable ? "YES" : "NO"}</td>
              <td className="mono">{orDash(c.default)}</td>
              <td>{c.primaryKey ? "PRIMARY" : ""}</td>
              <td>{c.autoIncrement ? "auto increment" : ""}</td>
              <td className="muted">{orDash(c.comment)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {t.kind === "table" && (
        <>
          <h4 className="block-title">Indexes</h4>
          <table className="list-table">
            <thead><tr><th>Name</th><th>Columns</th><th>Unique</th></tr></thead>
            <tbody>
              {t.indexes.map((i) => (
                <tr key={i.name}><td>{i.name}</td><td className="mono">{i.columns.join(", ")}</td><td>{i.unique ? "YES" : "NO"}</td></tr>
              ))}
              {t.indexes.length === 0 && <tr><td colSpan={3} className="muted">No indexes</td></tr>}
            </tbody>
          </table>
          <h4 className="block-title">Foreign Keys</h4>
          <table className="list-table">
            <thead><tr><th>Name</th><th>Columns</th><th>References</th><th>On Update</th><th>On Delete</th></tr></thead>
            <tbody>
              {t.foreignKeys.map((f) => (
                <tr key={f.name}>
                  <td>{f.name}</td>
                  <td className="mono">{f.columns.join(", ")}</td>
                  <td className="mono">{f.refTable}({f.refColumns.join(", ")})</td>
                  <td>{f.onUpdate}</td>
                  <td>{f.onDelete}</td>
                </tr>
              ))}
              {t.foreignKeys.length === 0 && <tr><td colSpan={5} className="muted">No foreign keys</td></tr>}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}

function InfoView({ tab }: { tab: TableTab }) {
  const info = useTableInfo(tab);
  const t = info.data;
  if (!t) return <div className="empty"><p>{info.isError ? (info.error as { message: string }).message : "Loading…"}</p></div>;
  return (
    <div className="table-scroll info-view">
      <dl className="kv wide">
        <dt>Name</dt><dd>{t.name}</dd>
        <dt>Type</dt><dd>{t.kind === "table" ? "Table" : "View"}</dd>
        <dt>Database</dt><dd>{tab.database}</dd>
        {t.kind === "table" && (<><dt>Rows</dt><dd>{formatCount(t.rows)}</dd></>)}
        {t.engine !== null && (<><dt>Engine</dt><dd>{t.engine}</dd></>)}
        {t.charset !== null && (<><dt>Charset</dt><dd>{t.charset}</dd></>)}
        {t.collation !== null && (<><dt>Collation</dt><dd>{t.collation}</dd></>)}
        <dt>Created</dt><dd>{orDash(t.created)}</dd>
        <dt>Modified</dt><dd>{orDash(t.modified)}</dd>
        <dt>Comment</dt><dd>{orDash(t.comment)}</dd>
      </dl>
      <div className="block-title row-between">
        <h4>{t.kind === "table" ? "CREATE TABLE" : "View definition"}</h4>
        <button className="btn sm" onClick={() => void navigator.clipboard.writeText(t.createSql).then(() => notify("Copied to clipboard"))}>
          <Copy size={13} />Copy
        </button>
      </div>
      <pre className="code-block">{t.createSql}</pre>
    </div>
  );
}
