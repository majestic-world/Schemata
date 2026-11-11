import { useQuery } from "@tanstack/react-query";
import { ChevronRight, Database, Server, Table2 } from "lucide-react";
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
    </footer>
  );
}
