import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, RefreshCw } from "lucide-react";
import { DatabaseIcon, TableIcon, ViewIcon } from "./icons";
import { api } from "../api/client";
import type { ObjectInfo } from "../api/types";
import { useWorkspace } from "../store/workspace";
import { formatCount, orDash } from "../lib/format";
import { useRefreshShortcut } from "../lib/refreshShortcut";
import { useRefetchFlash } from "../lib/refetchFlash";
import { ObjectContextMenu } from "./ObjectContextMenu";
import { isMod } from "../lib/platform";
import { dropObjects } from "../lib/objectActions";
import { useTabs } from "../store/tabs";
import { openTransfer } from "../store/transfer";
import { Menu, type MenuItem } from "./Menu";
import { createTypeahead } from "../lib/typeahead";

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
  const selectedObjects = useWorkspace((s) => s.selectedObjects);
  const selectObjects = useWorkspace((s) => s.selectObjects);
  const selectedNames = useMemo(() => new Set(selectedObjects.map((object) => object.name)), [selectedObjects]);
  const search = useWorkspace((s) => s.search.trim().toLowerCase());
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "name", desc: false });
  const tableRef = useRef<HTMLTableElement>(null);
  const [typeahead] = useState(createTypeahead);
  const connectionId = selection?.connectionId;
  const database = selection?.database;
  const objects = useQuery({
    queryKey: ["objects", connectionId, database],
    queryFn: () => api.listObjects(connectionId!, database!),
    enabled: !!connectionId && !!database,
  });
  useRefetchFlash(tableRef, `${connectionId}\u0000${database}`, objects.dataUpdatedAt, objects.isSuccess);
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
  useEffect(() => {
    if (!connectionId || !database || !objects.isSuccess || selectedObjects.length === 0) return;
    const visibleNames = new Set(rows.map((object) => object.name));
    const kept = selectedObjects.filter((object) => visibleNames.has(object.name));
    if (kept.length !== selectedObjects.length) selectObjects({ connectionId, database, objects: kept });
  }, [connectionId, database, objects.isSuccess, rows, selectedObjects, selectObjects]);

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
  const backgroundItems: MenuItem[] = [
    { label: "New Table", onSelect: () => useTabs.getState().newTable(connectionId, database) },
    { label: "Execute Sql File", onSelect: () => openTransfer({ kind: "runSql", connectionId, database }) },
    { label: "Dump Sql File", onSelect: () => openTransfer({ kind: "backup", connectionId, database, data: false, perTable: false }) },
    { label: "Dump Sql With Data", onSelect: () => openTransfer({ kind: "backup", connectionId, database, data: true, perTable: false }) },
  ];

  return (
    <>
      <div className="action-bar">
        {actions}
        <div className="grow" />
        <button className="icon-btn" title="Refresh" onClick={() => void qc.invalidateQueries({ queryKey: ["objects", connectionId, database] })}>
          <RefreshCw size={15} className={objects.isFetching ? "spin" : undefined} />
        </button>
      </div>
      <Menu items={backgroundItems} onOpenChange={(opened) => {
        if (opened) select({ connectionId, database });
      }}>
        <div className="table-scroll object-list" tabIndex={0} aria-busy={objects.isFetching} onKeyDown={(e) => {
          if (e.defaultPrevented || !e.currentTarget.contains(e.target as Node) || (e.target instanceof Element && e.target.closest('input, textarea, select, [contenteditable], [role="menu"], [role="dialog"]'))) return;
          if (isMod(e) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "a") {
            e.preventDefault();
            selectObjects({ connectionId, database, objects: rows.map(({ name, kind }) => ({ name, kind })) });
          } else if (e.key === "Delete" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && selectedObjects.length > 0) {
            e.preventDefault();
            if (!e.repeat) void dropObjects({ connectionId, database }, selectedObjects, qc);
          } else if (e.key === "Enter" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && selectedObjects.length === 1) {
            e.preventDefault();
            const object = rows.find((o) => o.name === selectedObjects[0].name);
            if (object && !e.repeat) onOpen(object);
          } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && !e.nativeEvent.isComposing) {
            const current = rows.findIndex((o) => o.name === selection?.object?.name);
            const index = typeahead(e.key, e.timeStamp, rows.map((o) => o.name), current);
            if (index === undefined) return;
            e.preventDefault();
            if (index < 0) return;
            const { name, kind } = rows[index];
            select({ connectionId, database, object: { name, kind } });
            tableRef.current?.querySelector(`tr[data-object="${CSS.escape(name)}"]`)?.scrollIntoView({ block: "nearest" });
          }
        }} onClick={(e) => {
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
                const selected = selectedNames.has(o.name);
                const Icon = o.kind === "table" ? TableIcon : ViewIcon;
                return (
                  <ObjectContextMenu key={o.name} object={{ connectionId, database, name: o.name, kind: o.kind }}>
                    <tr
                      className={selected ? "selected" : undefined}
                      aria-selected={selected}
                      onClick={(e) => {
                        if (isMod(e)) {
                          const next = selected
                            ? selectedObjects.filter((object) => object.name !== o.name)
                            : [...selectedObjects, { name: o.name, kind: o.kind }];
                          selectObjects({ connectionId, database, objects: next });
                        } else select({ connectionId, database, object: { name: o.name, kind: o.kind } });
                      }}
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
      </Menu>
    </>
  );
}
