import { useQuery } from "@tanstack/react-query";
import { ChevronRight, Rows3, ListTree, Info } from "lucide-react";
import { DatabaseIcon, EngineIcon, TableIcon, ViewIcon } from "./icons";
import { useTabs, type TableView } from "../store/tabs";
import { plural } from "../lib/format";
import { api } from "../api/client";
import { useWorkspace } from "../store/workspace";
import { useConnections } from "../store/connections";

export function StatusBar() {
  const selection = useWorkspace((s) => s.selection);
  const connections = useQuery({ queryKey: ["connections"], queryFn: api.listConnections });
  const open = useConnections((s) => s.open);
  const conn = connections.data?.find((c) => c.id === selection?.connectionId);
  const connected = !!conn && conn.id in open;
  return (
    <footer className="statusbar">
      <span className="status-item">
        <span className={`status-dot${connected ? " on" : ""}`} /> {connected ? "Connected" : "Not connected"}
      </span>
      {conn && (
        <nav className="crumbs" aria-label="Breadcrumb">
          <span><EngineIcon engine={conn.engine} size={14} />{conn.name}</span>
          {selection?.database && (<><ChevronRight size={12} className="sep" /><span><DatabaseIcon size={14} />{selection.database}</span></>)}
          {selection?.object && (
            <><ChevronRight size={12} className="sep" /><span>{selection.object.kind === "table" ? <TableIcon size={14} /> : <ViewIcon size={14} />}{selection.object.name}</span></>
          )}
        </nav>
      )}
      <div className="grow" />
      <TableToggles />
    </footer>
  );
}

const VIEWS: { view: TableView; label: string; icon: typeof Rows3 }[] = [
  { view: "data", label: "Data", icon: Rows3 },
  { view: "structure", label: "Structure", icon: ListTree },
  { view: "info", label: "Info", icon: Info },
];

function TableToggles() {
  const { tabs, active, setView, rowCounts } = useTabs();
  const tab = tabs.find((t) => t.id === active);
  if (!tab || tab.kind !== "table") return null;
  const rows = rowCounts[tab.id];
  return (
    <>
      <div className="view-toggle" role="group" aria-label="Table view">
        {VIEWS.map(({ view, label, icon: Icon }) => (
          <button key={view} className={tab.view === view ? "on" : ""} title={label} aria-label={label} onClick={() => setView(tab.id, view)}>
            <Icon size={14} />
          </button>
        ))}
      </div>
      <span className="status-item row-count">{rows === undefined ? "" : plural(rows, "row")}</span>
    </>
  );
}
