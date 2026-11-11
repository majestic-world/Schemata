import { useQuery } from "@tanstack/react-query";
import { Table2, Eye, Info, Layers, Database, Server, ChevronRight } from "lucide-react";
import { useTabs } from "../store/tabs";
import type { ReactNode } from "react";
import { api } from "../api/client";
import { ENGINE_LABEL } from "../api/types";
import { useWorkspace } from "../store/workspace";
import { useConnections } from "../store/connections";
import { formatCount, orDash, plural } from "../lib/format";

export function Inspector() {
  const selection = useWorkspace((s) => s.selection);
  if (!selection) {
    return <div className="card inspector-card"><div className="empty"><p>Select an object to see its details.</p></div></div>;
  }
  if (selection.object && selection.database) {
    return <ObjectDetails connectionId={selection.connectionId} database={selection.database} name={selection.object.name} />;
  }
  if (selection.database) return <DatabaseDetails connectionId={selection.connectionId} database={selection.database} />;
  return <ConnectionDetails connectionId={selection.connectionId} />;
}

function Heading({ icon, title, subtitle }: { icon: ReactNode; title: string; subtitle: string }) {
  return (
    <div className="inspector-head">
      <span className="inspector-icon">{icon}</span>
      <div className="inspector-title">
        <b title={title}>{title}</b>
        <span className="pill">{subtitle}</span>
      </div>
    </div>
  );
}

function Section({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <div className="card inspector-section">
      <div className="section-title">{icon}{title}</div>
      {children}
    </div>
  );
}

function ObjectDetails({ connectionId, database, name }: { connectionId: string; database: string; name: string }) {
  const info = useQuery({ queryKey: ["tableInfo", connectionId, database, name], queryFn: () => api.tableInfo(connectionId, database, name) });
  const t = info.data;
  if (info.isError) return <div className="card inspector-card"><div className="empty"><p>{(info.error as { message?: string }).message}</p></div></div>;
  if (!t) return <div className="card inspector-card"><div className="empty"><p>Loading…</p></div></div>;
  const isTable = t.kind === "table";
  return (
    <div className="inspector-stack">
      <Heading icon={isTable ? <Table2 size={22} /> : <Eye size={22} />} title={t.name} subtitle={isTable ? "Table" : "View"} />
      {isTable ? (
        <Section icon={<Info size={15} />} title="Table Information">
          <dl className="kv">
            <dt>Rows</dt><dd>{formatCount(t.rows)}</dd>
            <dt>Created Date</dt><dd>{orDash(t.created)}</dd>
            <dt>Modified Date</dt><dd>{orDash(t.modified)}</dd>
            <dt>Comment</dt><dd>{orDash(t.comment)}</dd>
          </dl>
        </Section>
      ) : (
        <Section icon={<Info size={15} />} title="Definition">
          <pre className="code-block">{t.createSql}</pre>
        </Section>
      )}
      <Section icon={<Layers size={15} />} title="Schema">
        <dl className="kv">
          <dt>Database</dt><dd>{database}</dd>
          {t.engine !== null && (<><dt>Engine</dt><dd><span className="status-dot on" /> {t.engine}</dd></>)}
          {t.charset !== null && (<><dt>Charset</dt><dd>{t.charset}</dd></>)}
          {t.collation !== null && (<><dt>Collation</dt><dd>{t.collation}</dd></>)}
          <dt>Columns</dt><dd>{t.columns.length}</dd>
          {isTable && (<><dt>Indexes</dt><dd>{t.indexes.length}</dd><dt>Foreign keys</dt><dd>{t.foreignKeys.length}</dd></>)}
        </dl>
      </Section>
      <button
        className="card inspector-summary"
        title={`Open ${t.name}`}
        onClick={() => useTabs.getState().openTable({ connectionId, database, name: t.name, objectKind: t.kind })}
      >
        <Database size={26} className="tree-icon" />
        <div className="grow">
          <b>{t.name}</b>
          <span>{isTable && t.rows !== null ? `${plural(t.rows, "row")} • ` : ""}{plural(t.columns.length, "column")}</span>
        </div>
        <ChevronRight size={16} className="faint" />
      </button>
    </div>
  );
}

function DatabaseDetails({ connectionId, database }: { connectionId: string; database: string }) {
  const objects = useQuery({ queryKey: ["objects", connectionId, database], queryFn: () => api.listObjects(connectionId, database) });
  const tables = objects.data?.filter((o) => o.kind === "table").length;
  const views = objects.data?.filter((o) => o.kind === "view").length;
  return (
    <div className="inspector-stack">
      <Heading icon={<Database size={22} />} title={database} subtitle="Database" />
      <Section icon={<Info size={15} />} title="Contents">
        <dl className="kv">
          <dt>Tables</dt><dd>{formatCount(tables)}</dd>
          <dt>Views</dt><dd>{formatCount(views)}</dd>
        </dl>
      </Section>
    </div>
  );
}

function ConnectionDetails({ connectionId }: { connectionId: string }) {
  const connections = useQuery({ queryKey: ["connections"], queryFn: api.listConnections });
  const version = useConnections((s) => s.open[connectionId]);
  const c = connections.data?.find((x) => x.id === connectionId);
  if (!c) return null;
  return (
    <div className="inspector-stack">
      <Heading icon={<Server size={22} />} title={c.name} subtitle={ENGINE_LABEL[c.engine]} />
      <Section icon={<Info size={15} />} title="Connection">
        <dl className="kv">
          <dt>Status</dt><dd><span className={`status-dot${version !== undefined ? " on" : ""}`} /> {version !== undefined ? "Connected" : "Closed"}</dd>
          {version ? (<><dt>Server</dt><dd>{version}</dd></>) : null}
          {c.engine === "sqlite" ? (
            <><dt>File</dt><dd>{c.filePath}</dd></>
          ) : (
            <><dt>Host</dt><dd>{c.host}:{c.port}</dd><dt>User</dt><dd>{c.user}</dd></>
          )}
        </dl>
      </Section>
    </div>
  );
}
