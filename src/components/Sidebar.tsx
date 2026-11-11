import { Plus } from "lucide-react";

export function Sidebar() {
  return (
    <>
      <div className="panel-header">
        <span className="grow">Connections</span>
        <button className="icon-btn" title="New connection"><Plus size={16} /></button>
      </div>
      <div className="empty">
        <p>No connections yet. Create one to get started.</p>
      </div>
    </>
  );
}
