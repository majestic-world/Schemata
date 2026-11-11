import { useQueryClient } from "@tanstack/react-query";
import { ContextMenu, DropdownMenu } from "radix-ui";
import { X, LayoutGrid, PencilRuler, Trash2, MoreHorizontal, FileCode2, ExternalLink, Plus } from "lucide-react";
import type { ReactNode } from "react";
import { TableIcon, ViewIcon } from "./icons";
import { QueryEditor } from "./QueryEditor";
import { ObjectList } from "./ObjectList";
import { TableTabView } from "./TableTabView";
import { TableDesigner } from "./TableDesigner";
import { OBJECTS_TAB, useTabs, type NewTableTab, type Tab } from "../store/tabs";
import { useWorkspace } from "../store/workspace";
import { useConnection } from "../lib/useConnection";
import { copyCreateStatement, designTable, dropObjects, renameTable, truncateTable } from "../lib/objectActions";
import { openTransfer } from "../store/transfer";
import { requestCloseTabs } from "../lib/tabActions";

function TabContextMenu({ id, children }: { id: string; children: ReactNode }) {
  const tabs = useTabs((s) => s.tabs);
  const index = tabs.findIndex((tab) => tab.id === id);
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild>{children}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="menu">
          <ContextMenu.Item className="menu-item" disabled={id === OBJECTS_TAB} onSelect={() => void requestCloseTabs([id])}>Close Tab</ContextMenu.Item>
          <ContextMenu.Item className="menu-item" disabled={!tabs.some((tab) => tab.id !== id)} onSelect={() => void requestCloseTabs(tabs.filter((tab) => tab.id !== id).map((tab) => tab.id))}>Close Other Tabs</ContextMenu.Item>
          <ContextMenu.Item className="menu-item" disabled={index === tabs.length - 1} onSelect={() => void requestCloseTabs(tabs.slice(index + 1).map((tab) => tab.id))}>Close Tabs to the Right</ContextMenu.Item>
          <ContextMenu.Separator className="menu-sep" />
          <ContextMenu.Item className="menu-item" disabled={tabs.length === 0} onSelect={() => void requestCloseTabs(tabs.map((tab) => tab.id))}>Close All Tabs</ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

function tabLabel(t: Tab) {
  switch (t.kind) {
    case "table":
      return `${t.name}@${t.database}`;
    case "newTable":
      return `New table@${t.database}`;
    case "query":
      return t.title;
    default: {
      const never: never = t;
      return never;
    }
  }
}

export function Workspace() {
  const { tabs, active, activate } = useTabs();
  const activeTab = tabs.find((t) => t.id === active);
  return (
    <>
      <div className="tabs" role="tablist">
        <TabContextMenu id={OBJECTS_TAB}>
          <div className={`tab${active === OBJECTS_TAB ? " active" : ""}`} role="tab" tabIndex={0} onClick={() => activate(OBJECTS_TAB)}>
            <LayoutGrid size={14} />
            <span className="title">Objects</span>
          </div>
        </TabContextMenu>
        {tabs.map((t) => {
          const icon =
            t.kind === "newTable" ? <PencilRuler size={14} className="tab-glyph design" /> : t.kind === "query" ? <FileCode2 size={14} className="tab-glyph query" /> : t.objectKind === "table" ? <TableIcon size={15} /> : <ViewIcon size={15} />;
          return (
            <TabContextMenu key={t.id} id={t.id}>
              <div
                className={`tab${active === t.id ? " active" : ""}`}
                role="tab"
                tabIndex={0}
                title={tabLabel(t)}
                onClick={() => {
                  activate(t.id);
                  if (t.kind === "table") useWorkspace.getState().select({ connectionId: t.connectionId, database: t.database, object: { name: t.name, kind: t.objectKind } });
                }}
                onAuxClick={(e) => e.button === 1 && void requestCloseTabs([t.id])}
              >
                {icon}
                <span className="title">{tabLabel(t)}</span>
                {t.kind === "query" && t.sql !== t.savedSql && <span className="dirty" title="Unsaved changes" />}
                <button className="icon-btn tab-close" aria-label="Close tab" onClick={(e) => { e.stopPropagation(); void requestCloseTabs([t.id]); }}>
                  <X size={16} />
                </button>
              </div>
            </TabContextMenu>
          );
        })}
      </div>
      <div className="tab-body">
        {!activeTab ? (
          <ObjectsView />
        ) : activeTab.kind === "table" ? (
          <TableTabView key={activeTab.id} tab={activeTab} />
        ) : activeTab.kind === "query" ? (
          <QueryEditor key={activeTab.id} tab={activeTab} />
        ) : (
          <NewTableView key={activeTab.id} tab={activeTab} />
        )}
      </div>
    </>
  );
}

function NewTableView({ tab }: { tab: NewTableTab }) {
  const conn = useConnection(tab.connectionId);
  if (!conn) return null;
  return (
    <TableDesigner
      connectionId={tab.connectionId}
      database={tab.database}
      engine={conn.engine}
      onSaved={(name) => useTabs.getState().replace(tab.id, { connectionId: tab.connectionId, database: tab.database, name, objectKind: "table" }, "structure")}
    />
  );
}

function ObjectsView() {
  const qc = useQueryClient();
  const selection = useWorkspace((s) => s.selection);
  const selectedObjects = useWorkspace((s) => s.selectedObjects);
  const multiple = selectedObjects.length > 1;
  const { openTable, newTable } = useTabs();
  const obj = selection?.object;
  const ref = selection?.database && obj ? { connectionId: selection.connectionId, database: selection.database, name: obj.name, kind: obj.kind } : null;
  const isTable = obj?.kind === "table";
  return (
    <ObjectList
      onOpen={(o) => selection?.database && openTable({ connectionId: selection.connectionId, database: selection.database, name: o.name, objectKind: o.kind })}
      actions={
        <>
          <button className="obj-btn" disabled={!ref || multiple} onClick={() => ref && openTable({ connectionId: ref.connectionId, database: ref.database, name: ref.name, objectKind: ref.kind })}>
            <ExternalLink size={15} className="ob-open" />Open {obj?.kind === "view" ? "View" : "Table"}
          </button>
          <button className="obj-btn" disabled={!ref || !isTable || multiple} onClick={() => ref && designTable(ref)}><PencilRuler size={15} className="ob-design" />Design Table</button>
          <button className="obj-btn" disabled={!selection?.database} onClick={() => selection?.database && newTable(selection.connectionId, selection.database)}><Plus size={15} className="ob-new" />New Table</button>
          <button className="obj-btn" disabled={!selection?.database || selectedObjects.length === 0} onClick={() => selection?.database && void dropObjects({ connectionId: selection.connectionId, database: selection.database }, selectedObjects, qc)}><Trash2 size={15} className="ob-delete" />{multiple ? `Delete ${selectedObjects.length} Objects` : `Delete ${obj?.kind === "view" ? "View" : "Table"}`}</button>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button className="obj-btn" aria-label="More actions" disabled={!ref || multiple}><MoreHorizontal size={16} /></button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content className="menu" align="start">
                <DropdownMenu.Item className="menu-item" disabled={!isTable} onSelect={() => ref && void renameTable(ref, qc)}>Rename Table… (F6)</DropdownMenu.Item>
                <DropdownMenu.Item className="menu-item" onSelect={() => ref && void copyCreateStatement(ref)}>Copy CREATE Statement</DropdownMenu.Item>
                <DropdownMenu.Separator className="menu-sep" />
                <DropdownMenu.Item className="menu-item" onSelect={() => ref && openTransfer({ kind: "export", connectionId: ref.connectionId, database: ref.database, source: { kind: "table", table: ref.name }, label: ref.name })}>Export…</DropdownMenu.Item>
                <DropdownMenu.Item className="menu-item" disabled={!isTable} onSelect={() => ref && openTransfer({ kind: "import", connectionId: ref.connectionId, database: ref.database, table: ref.name })}>Import CSV…</DropdownMenu.Item>
                <DropdownMenu.Item className="menu-item" disabled={!isTable} onSelect={() => ref && openTransfer({ kind: "generate", connectionId: ref.connectionId, database: ref.database, table: ref.name })}>Generate Data…</DropdownMenu.Item>
                <DropdownMenu.Separator className="menu-sep" />
                <DropdownMenu.Item className="menu-item danger" disabled={!isTable} onSelect={() => ref && void truncateTable(ref, qc)}>Empty Table…</DropdownMenu.Item>
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </>
      }
    />
  );
}
