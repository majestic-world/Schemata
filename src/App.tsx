import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { OBJECTS_TAB, useTabs } from "./store/tabs";
import { useWorkspace } from "./store/workspace";
import { isMod, isWindows } from "./lib/platform";
import { isRefreshShortcut, runRefreshHandler } from "./lib/refreshShortcut";
import { notifyError } from "./store/toasts";
import { copyTable, pasteTable } from "./store/tableClipboard";
import { designTable, renameTable } from "./lib/objectActions";
import { Toolbar } from "./components/Toolbar";
import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { Workspace } from "./components/Workspace";
import { Inspector } from "./components/Inspector";
import { StatusBar } from "./components/StatusBar";
import { Splitter } from "./components/Splitter";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { PasswordPrompt } from "./components/PasswordPrompt";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { Toaster } from "./components/Toaster";
import { TransferDialogs } from "./components/TransferDialogs";
import { AttachDatabaseDialog, CreateDatabaseDialog } from "./components/DatabaseDialogs";
import { useLayout } from "./store/layout";
import { useConnections } from "./store/connections";
import { api } from "./api/client";

export function App() {
  const layout = useLayout();

  const qc = useQueryClient();
  const activeTab = useTabs((s) => s.active);

  useEffect(() => {
    let disposed = false;
    let frame: number | undefined;
    // Wait for the committed shell, fonts and a rendered frame, not remote connections.
    void document.fonts.ready.then(() => {
      if (disposed) return;
      frame = requestAnimationFrame(() => {
        frame = requestAnimationFrame(() => void getCurrentWindow().show());
      });
    });
    return () => {
      disposed = true;
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, []);

  // Sessions outlive a webview reload; resync which connections are open. On a fresh start, reopen the
  // connections that were open at exit (silently: one that needs a password simply stays closed).
  useEffect(() => {
    void (async () => {
      const ids = await api.openConnectionIds();
      ids.forEach((id) => useConnections.getState().markOpen(id, ""));
      if (ids.length > 0) return;
      for (const id of useWorkspace.getState().openAtExit) {
        await api.openConnection(id, null).then((v) => useConnections.getState().markOpen(id, v)).catch(() => undefined);
      }
    })();
    return useConnections.subscribe((s) => useWorkspace.getState().setOpenAtExit(Object.keys(s.open)));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isRefreshShortcut(e)) {
        // Never reload the webview: refresh only the data of the focused area.
        e.preventDefault();
        if (e.repeat) return;
        if (document.activeElement?.closest(".sidebar")) {
          void qc.invalidateQueries({ queryKey: ["connections"] });
          void qc.invalidateQueries({ queryKey: ["databases"] });
          void qc.invalidateQueries({ queryKey: ["objects"] });
        } else {
          runRefreshHandler();
        }
        return;
      }
      if (e.key === "F6" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
        if (
          e.defaultPrevented || e.repeat ||
          useTabs.getState().active !== OBJECTS_TAB ||
          document.querySelector('[role="dialog"], [role="menu"]') ||
          (e.target instanceof HTMLElement && e.target.closest('input, textarea, select, [contenteditable="true"]'))
        ) return;
        const selection = useWorkspace.getState().selection;
        if (selection?.database && selection.object?.kind === "table") {
          e.preventDefault();
          void renameTable({ connectionId: selection.connectionId, database: selection.database, name: selection.object.name, kind: "table" }, qc);
        }
        return;
      }
      if (!isMod(e)) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        document.getElementById("global-search")?.focus();
      } else if (key === "t") {
        e.preventDefault();
        const sel = useWorkspace.getState().selection;
        const open = Object.keys(useConnections.getState().open);
        const connectionId = sel && sel.connectionId in useConnections.getState().open ? sel.connectionId : open[0];
        if (connectionId) useTabs.getState().newQuery({ connectionId, database: sel?.connectionId === connectionId ? (sel.database ?? null) : null });
        else notifyError("No open connection", { message: "Open a connection to start a query." });
      } else if (key === "d" && !e.shiftKey) {
        // Navicat: Ctrl+D designs the open table, or the table selected in the tree / object list.
        const tabs = useTabs.getState();
        const tab = tabs.tabs.find((t) => t.id === tabs.active);
        const sel = useWorkspace.getState().selection;
        if (tab?.kind === "table") {
          e.preventDefault();
          tabs.setView(tab.id, "structure");
        } else if (!tab && sel?.database && sel.object?.kind === "table") {
          e.preventDefault();
          designTable({ connectionId: sel.connectionId, database: sel.database, name: sel.object.name, kind: "table" });
        }
      } else if (
        (key === "c" || key === "v") &&
        !e.shiftKey &&
        e.target instanceof HTMLElement &&
        !e.target.closest("input, textarea, select") &&
        e.target.closest(".sidebar, .object-list")
      ) {
        // Navicat-style: copy a table in the tree or object list, paste it into another database.
        const sel = useWorkspace.getState().selection;
        if (key === "c" && sel?.database && sel.object?.kind === "table") {
          e.preventDefault();
          copyTable({ connectionId: sel.connectionId, database: sel.database, name: sel.object.name });
        } else if (key === "v" && sel?.database) {
          e.preventDefault();
          pasteTable({ connectionId: sel.connectionId, database: sel.database });
        }
      }
    };
    const onRejection = (e: PromiseRejectionEvent) => notifyError("Unexpected error", e.reason);
    // Custom Radix context menus still handle the event; only the webview's default is suppressed.
    const onContextMenu = (e: MouseEvent) => e.preventDefault();
    window.addEventListener("keydown", onKey);
    window.addEventListener("unhandledrejection", onRejection);
    window.addEventListener("contextmenu", onContextMenu);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("unhandledrejection", onRejection);
      window.removeEventListener("contextmenu", onContextMenu);
    };
  }, [qc]);

  return (
    <div className="app">
      {isWindows && <TitleBar />}
      <Toolbar />
      <div className="app-body">
        <aside className="panel sidebar" style={{ width: layout.sidebarWidth }}>
          <Sidebar />
        </aside>
        <Splitter side="left" value={layout.sidebarWidth} onChange={layout.setSidebarWidth} min={200} max={520} />
        <main className="panel workspace">
          <ErrorBoundary resetKey={activeTab}>
            <Workspace />
          </ErrorBoundary>
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
      <CreateDatabaseDialog />
      <AttachDatabaseDialog />
      <TransferDialogs />
      <Toaster />
    </div>
  );
}
