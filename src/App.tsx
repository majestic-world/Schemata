import { useEffect } from "react";
import { Toolbar } from "./components/Toolbar";
import { Sidebar } from "./components/Sidebar";
import { Workspace } from "./components/Workspace";
import { Inspector } from "./components/Inspector";
import { StatusBar } from "./components/StatusBar";
import { Splitter } from "./components/Splitter";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { PasswordPrompt } from "./components/PasswordPrompt";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { Toaster } from "./components/Toaster";
import { useLayout } from "./store/layout";
import { useConnections } from "./store/connections";
import { api } from "./api/client";

export function App() {
  const layout = useLayout();

  // Sessions outlive a webview reload; resync which connections are open.
  useEffect(() => {
    void api.openConnectionIds().then((ids) => ids.forEach((id) => useConnections.getState().markOpen(id, "")));
  }, []);

  return (
    <div className="app">
      <Toolbar />
      <div className="app-body">
        <aside className="panel sidebar" style={{ width: layout.sidebarWidth }}>
          <Sidebar />
        </aside>
        <Splitter side="left" value={layout.sidebarWidth} onChange={layout.setSidebarWidth} min={200} max={520} />
        <main className="panel workspace">
          <Workspace />
        </main>
        {layout.inspectorOpen && (
          <>
            <Splitter side="right" value={layout.inspectorWidth} onChange={layout.setInspectorWidth} min={260} max={560} />
            <aside className="panel inspector" style={{ width: layout.inspectorWidth }}>
              <Inspector />
            </aside>
          </>
        )}
      </div>
      <StatusBar />
      <ConnectionDialog />
      <PasswordPrompt />
      <ConfirmDialog />
      <Toaster />
    </div>
  );
}
