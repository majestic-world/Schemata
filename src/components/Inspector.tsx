import { useQuery } from "@tanstack/react-query";
import { Copy, ExternalLink } from "lucide-react";
import type { ReactNode } from "react";
import { api } from "../api/client";
import { ENGINE_LABEL, type Engine } from "../api/types";
import { useWorkspace } from "../store/workspace";
import { useConnections } from "../store/connections";
import { useTabs } from "../store/tabs";
import { useConnection } from "../lib/useConnection";
import { formatCount, orDash, plural } from "../lib/format";
import { notify } from "../store/toasts";
import { DatabaseIcon, EngineIcon, TableIcon, ViewIcon } from "./icons";

export function Inspector() {
  const selection = useWorkspace((s) => s.selection);
  if (!selection) return <div className="insp-empty">Select a connection, database or table to see its details.</div>;
  if (selection.object && selection.database) {
    return <ObjectDetails connectionId={selection.connectionId} database={selection.database} name={selection.object.name} />;
  }
  if (selection.database) return <DatabaseDetails connectionId={selection.connectionId} database={selection.database} />;
  return <ConnectionDetails connectionId={selection.connectionId} />;
}

function Head({ icon, title, kind, action }: { icon: ReactNode; title: string; kind: string; action?: ReactNode }) {
  return (
    <div className="insp-head">
      <span className="insp-glyph">{icon}</span>
      <div className="insp-title">
        <h2 title={title}>{title}</h2>
        <span>{kind}</span>
        {action}
      </div>
    </div>
  );
}

/** Where the object lives: connection, then database. */
function Location({ connectionId, database }: { connectionId: string; database?: string }) {
  const conn = useConnection(connectionId);
  if (!conn) return null;
  return (
    <ul className="insp-location">
      <li><EngineIcon engine={conn.engine} size={15} />{conn.name}</li>
      {database && <li><DatabaseIcon size={15} />{database}</li>}
    </ul>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="insp-field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

function ObjectDetails({ connectionId, database, name }: { connectionId: string; database: string; name: string }) {
  const info = useQuery({ queryKey: ["tableInfo", connectionId, database, name], queryFn: () => api.tableInfo(connectionId, database, name) });
  const t = info.data;
  if (info.isError) return <div className="insp-empty error-text">{(info.error as { message?: string }).message}</div>;
  if (!t) return <div className="insp-empty">Loading…</div>;
  const isTable = t.kind === "table";
  const open = () => useTabs.getState().openTable({ connectionId, database, name: t.name, objectKind: t.kind });
  return (
    <div className="insp">
      <Head
        icon={isTable ? <TableIcon size={52} /> : <ViewIcon size={52} />}
        title={t.name}
        kind={isTable ? "Table" : "View"}
        action={<button className="link-btn" onClick={open}><ExternalLink size={13} />Open {isTable ? "table" : "view"}</button>}
      />
      <Location connectionId={connectionId} database={database} />
      <dl className="insp-fields">
        {isTable && <Field label="Rows">{formatCount(t.rows)}</Field>}
        <Field label="Columns">{plural(t.columns.length, "column")}</Field>
        {isTable && <Field label="Indexes and keys">{plural(t.indexes.length, "index", "indexes")} · {plural(t.foreignKeys.length, "foreign key")}</Field>}
        {t.engine !== null && <Field label="Engine">{t.engine}</Field>}
        {t.collation !== null && <Field label="Collation">{t.collation}{t.charset ? <span className="faint"> ({t.charset})</span> : null}</Field>}
        {(t.created !== null || isTable) && <Field label="Created date">{orDash(t.created)}</Field>}
        {(t.modified !== null || isTable) && <Field label="Modified date">{orDash(t.modified)}</Field>}
        {isTable && <Field label="Comment">{orDash(t.comment)}</Field>}
      </dl>
      {!isTable && (
        <div className="insp-ddl">
          <div className="insp-ddl-head">
            <span>Definition</span>
            <button className="link-btn" onClick={() => void navigator.clipboard.writeText(t.createSql).then(() => notify("Definition copied"))}><Copy size={12} />Copy</button>
          </div>
          <pre className="code-block">{t.createSql}</pre>
        </div>
      )}
    </div>
  );
}

function DatabaseDetails({ connectionId, database }: { connectionId: string; database: string }) {
  const objects = useQuery({ queryKey: ["objects", connectionId, database], queryFn: () => api.listObjects(connectionId, database) });
  const tables = objects.data?.filter((o) => o.kind === "table") ?? [];
  const views = objects.data?.filter((o) => o.kind === "view").length;
  const rows = tables.reduce((sum, t) => sum + (t.rows ?? 0), 0);
  return (
    <div className="insp">
      <Head icon={<DatabaseIcon size={52} />} title={database} kind="Database" />
      <Location connectionId={connectionId} />
      <dl className="insp-fields">
        <Field label="Tables">{formatCount(objects.data ? tables.length : undefined)}</Field>
        <Field label="Views">{formatCount(views)}</Field>
        <Field label="Rows (approximate)">{objects.data ? formatCount(rows) : "—"}</Field>
      </dl>
    </div>
  );
}

function ConnectionDetails({ connectionId }: { connectionId: string }) {
  const c = useConnection(connectionId);
  const version = useConnections((s) => s.open[connectionId]);
  if (!c) return null;
  const engine: Engine = c.engine;
  return (
    <div className="insp">
      <Head icon={<EngineIcon engine={engine} size={52} />} title={c.name} kind={`${ENGINE_LABEL[engine]} connection`} />
      <dl className="insp-fields">
        <Field label="Status"><span className={`status-dot${version !== undefined ? " on" : ""}`} /> {version !== undefined ? "Connected" : "Closed"}</Field>
        {version ? <Field label="Server version">{version}</Field> : null}
        {c.engine === "sqlite" ? (
          <Field label="Database file">{c.filePath}</Field>
        ) : (
          <>
            <Field label="Host">{c.host}:{c.port}</Field>
            <Field label="User">{c.user}</Field>
            <Field label="Password">{c.savePassword ? "Saved in the system credential store" : "Asked when connecting"}</Field>
          </>
        )}
      </dl>
    </div>
  );
}
