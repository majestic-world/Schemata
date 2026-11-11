import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, RefreshCw } from "lucide-react";
import { DatabaseIcon, TableIcon, ViewIcon } from "./icons";
import { api } from "../api/client";
import type { ObjectInfo } from "../api/types";
import { useWorkspace } from "../store/workspace";
import { formatCount, orDash } from "../lib/format";
import { useRefreshShortcut } from "../lib/refreshShortcut";
import { ObjectContextMenu } from "./ObjectContextMenu";

type SortKey = "name" | "rows" | "modified" | "comment";

/** Tables always precede views; `key` orders within each kind. */
function compare(a: ObjectInfo, b: ObjectInfo, key: SortKey, desc: boolean): number {
  if (a.kind !== b.kind) return a.kind === "table" ? -1 : 1;
  return (desc ? -1 : 1) * compareBy(a, b, key);
}

function compareBy(a: ObjectInfo, b: ObjectInfo, key: SortKey): number {
  switch (key) {
    case "rows":
      return (a.rows ?? -1) - (b.rows ?? -1);
    case "modified":
      return (a.modified ?? "").localeCompare(b.modified ?? "");
    case "comment":
      return (a.comment ?? "").localeCompare(b.comment ?? "");
    case "name":
      return a.name.localeCompare(b.name);
    default: {
      const never: never = key;
      return never;
    }
  }
}

export function ObjectList({ actions, onOpen }: { actions?: React.ReactNode; onOpen: (o: ObjectInfo) => void }) {
  const qc = useQueryClient();
  const selection = useWorkspace((s) => s.selection);
  const select = useWorkspace((s) => s.select);
  const search = useWorkspace((s) => s.search.trim().toLowerCase());
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "name", desc: false });
  const tableRef = useRef<HTMLTableElement>(null);
  const connectionId = selection?.connectionId;
  const database = selection?.database;
  const objects = useQuery({
    queryKey: ["objects", connectionId, database],
    queryFn: () => api.listObjects(connectionId!, database!),
    enabled: !!connectionId && !!database,
  });
  const lastUpdate = useRef({ connectionId, database, at: objects.dataUpdatedAt });
  useEffect(() => {
    const previous = lastUpdate.current;
    lastUpdate.current = { connectionId, database, at: objects.dataUpdatedAt };
    if (
      previous.connectionId !== connectionId || previous.database !== database ||
      previous.at === 0 || previous.at === objects.dataUpdatedAt || !objects.isSuccess
    ) return;

    // A completed refetch flashes even when structural sharing keeps the same rows.
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const animation = tableRef.current?.animate(
      [{ opacity: reducedMotion ? 0.8 : 0.35 }, { opacity: 1 }],
      { duration: reducedMotion ? 120 : 240, easing: "ease-out" },
    );
    return () => animation?.cancel();
  }, [connectionId, database, objects.dataUpdatedAt, objects.isSuccess]);
  useRefreshShortcut(() => {
    if (!connectionId) return;
    if (!database) {
      void qc.invalidateQueries({ queryKey: ["databases", connectionId] });
      return;
    }
    void qc.invalidateQueries({ queryKey: ["objects", connectionId, database] });
    void qc.invalidateQueries({ queryKey: ["tableInfo", connectionId, database] });
  });

  const rows = useMemo(() => {
    const list = (objects.data ?? []).filter((o) => !search || o.name.toLowerCase().includes(search));
    return [...list].sort((a, b) => compare(a, b, sort.key, sort.desc));
  }, [objects.data, sort, search]);

  if (!connectionId || !database) {
    return (
      <div className="empty">
        <DatabaseIcon size={44} />
        <h3>No database selected</h3>
        <p>Select a database in the sidebar to see its tables and views.</p>
      </div>
    );
  }

  const header = (key: SortKey, label: string, align?: "right") => (
    <th
      className={align === "right" ? "num" : undefined}
      onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : false }))}
      aria-sort={sort.key === key ? (sort.desc ? "descending" : "ascending") : "none"}
    >
      <span className="th">
        {label}
        {sort.key === key && (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
      </span>
    </th>
  );

  return (
    <>
      <div className="action-bar">
        {actions}
        <div className="grow" />
        <button className="icon-btn" title="Refresh" onClick={() => void qc.invalidateQueries({ queryKey: ["objects", connectionId, database] })}>
          <RefreshCw size={15} className={objects.isFetching ? "spin" : undefined} />
        </button>
      </div>
      <div className="table-scroll object-list" tabIndex={-1} aria-busy={objects.isFetching} onClick={(e) => {
        if (selection?.object && e.target instanceof Element && e.currentTarget.contains(e.target) && !e.target.closest("tbody tr")) {
          select({ connectionId, database });
        }
      }}>
        <table ref={tableRef} className="list-table">
          <thead>
            <tr>
              {header("name", "Name")}
              {header("rows", "Rows", "right")}
              {header("modified", "Modified Date")}
              {header("comment", "Comment")}
            </tr>
          </thead>
          <tbody>
            {rows.map((o) => {
              const selected = selection?.object?.name === o.name;
              const Icon = o.kind === "table" ? TableIcon : ViewIcon;
              return (
                <ObjectContextMenu key={o.name} object={{ connectionId, database, name: o.name, kind: o.kind }}>
                  <tr
                    className={selected ? "selected" : undefined}
                    onClick={() => select({ connectionId, database, object: { name: o.name, kind: o.kind } })}
                    onDoubleClick={() => onOpen(o)}
                    data-object={o.name}
                  >
                    <td><span className="cell-name"><Icon size={16} />{o.name}</span></td>
                    <td className="num">{formatCount(o.rows)}</td>
                    <td>{orDash(o.modified)}</td>
                    <td className="muted">{orDash(o.comment)}</td>
                  </tr>
                </ObjectContextMenu>
              );
            })}
          </tbody>
        </table>
        {objects.isSuccess && rows.length === 0 && <div className="empty"><p>{search ? "No objects match the search." : "This database has no tables or views."}</p></div>}
      </div>
    </>
  );
}
