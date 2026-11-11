import { Database } from "lucide-react";

export function Workspace() {
  return (
    <>
      <div className="tabs">
        <div className="tab active"><span className="title">Objects</span></div>
      </div>
      <div className="tab-body">
        <div className="empty">
          <Database size={36} strokeWidth={1.5} />
          <h3>Nothing open</h3>
          <p>Select a database in the sidebar to see its objects.</p>
        </div>
      </div>
    </>
  );
}
