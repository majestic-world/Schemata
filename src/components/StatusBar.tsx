import { useQuery } from "@tanstack/react-query";
import { ChevronRight, Database, Server, Table2, Rows3, ListTree, Info } from "lucide-react";
import { useTabs, type TableView } from "../store/tabs";
import { formatCount } from "../lib/format";
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
          <span><Server size={14} /> {conn.name}</span>
          {selection?.database && (<><ChevronRight size={13} className="sep" /><span><Database size={14} /> {selection.database}</span></>)}
          {selection?.object && (<><ChevronRight size={13} className="sep" /><span><Table2 size={14} /> {selection.object.name}</span></>)}
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
      <div className="segmented" role="group" aria-label="Table view">
        {VIEWS.map(({ view, label, icon: Icon }) => (
          <button key={view} className={tab.view === view ? "on" : ""} onClick={() => setView(tab.id, view)}>
            <Icon size={14} />{label}
          </button>
        ))}
      </div>
      <span className="status-item row-count">{rows === undefined ? "" : `${formatCount(rows)} row${rows === 1 ? "" : "s"}`}</span>
    </>
  );
}
