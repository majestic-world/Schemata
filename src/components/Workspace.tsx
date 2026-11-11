import { useQueryClient } from "@tanstack/react-query";
import { DropdownMenu } from "radix-ui";
import { X, Table2, Eye, LayoutGrid, PencilRuler, Plus, Trash2, MoreHorizontal, FileCode2 } from "lucide-react";
import { QueryEditor } from "./QueryEditor";
import { ObjectList } from "./ObjectList";
import { TableTabView } from "./TableTabView";
import { TableDesigner } from "./TableDesigner";
import { OBJECTS_TAB, useTabs, type NewTableTab, type Tab } from "../store/tabs";
import { useWorkspace } from "../store/workspace";
import { useConnection } from "../lib/useConnection";
import { copyCreateStatement, designTable, dropObject, renameTable, truncateTable } from "../lib/objectActions";
import { openTransfer } from "../store/transfer";

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
  const { tabs, active, activate, close } = useTabs();
  const activeTab = tabs.find((t) => t.id === active);
  return (
    <>
      <div className="tabs" role="tablist">
        <div className={`tab${active === OBJECTS_TAB ? " active" : ""}`} role="tab" onClick={() => activate(OBJECTS_TAB)}>
          <LayoutGrid size={14} />
          <span className="title">Objects</span>
        </div>
        {tabs.map((t) => {
          const Icon = t.kind === "newTable" ? PencilRuler : t.kind === "query" ? FileCode2 : t.objectKind === "table" ? Table2 : Eye;
          return (
            <div
              key={t.id}
              className={`tab${active === t.id ? " active" : ""}`}
              role="tab"
              title={tabLabel(t)}
              onClick={() => {
                activate(t.id);
                if (t.kind === "table") useWorkspace.getState().select({ connectionId: t.connectionId, database: t.database, object: { name: t.name, kind: t.objectKind } });
              }}
              onAuxClick={(e) => e.button === 1 && close(t.id)}
            >
              <Icon size={14} className="tree-icon obj" />
              <span className="title">{tabLabel(t)}</span>
              {t.kind === "query" && t.sql !== t.savedSql && <span className="dirty" title="Unsaved changes" />}
              <button className="icon-btn tab-close" aria-label="Close tab" onClick={(e) => { e.stopPropagation(); close(t.id); }}>
                <X size={13} />
              </button>
            </div>
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
  const { openTable, newTable } = useTabs();
  const obj = selection?.object;
  const ref = selection?.database && obj ? { connectionId: selection.connectionId, database: selection.database, name: obj.name, kind: obj.kind } : null;
  const isTable = obj?.kind === "table";
  return (
    <ObjectList
      onOpen={(o) => selection?.database && openTable({ connectionId: selection.connectionId, database: selection.database, name: o.name, objectKind: o.kind })}
      actions={
        <>
          <button className="btn primary" disabled={!ref} onClick={() => ref && openTable({ connectionId: ref.connectionId, database: ref.database, name: ref.name, objectKind: ref.kind })}>
            {obj?.kind === "view" ? <Eye size={15} /> : <Table2 size={15} />}Open {obj?.kind === "view" ? "View" : "Table"}
          </button>
          <button className="btn" disabled={!ref || !isTable} onClick={() => ref && designTable(ref)}><PencilRuler size={15} />Design Table</button>
          <button className="btn" disabled={!selection?.database} onClick={() => selection?.database && newTable(selection.connectionId, selection.database)}><Plus size={15} />New Table</button>
          <button className="btn danger" disabled={!ref} onClick={() => ref && void dropObject(ref, qc)}><Trash2 size={15} />Delete {obj?.kind === "view" ? "View" : "Table"}</button>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button className="icon-btn" aria-label="More actions" disabled={!ref}><MoreHorizontal size={16} /></button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content className="menu" align="start">
                <DropdownMenu.Item className="menu-item" disabled={!isTable} onSelect={() => ref && void renameTable(ref, qc)}>Rename Table…</DropdownMenu.Item>
                <DropdownMenu.Item className="menu-item" onSelect={() => ref && void copyCreateStatement(ref)}>Copy CREATE Statement</DropdownMenu.Item>
                <DropdownMenu.Separator className="menu-sep" />
                <DropdownMenu.Item className="menu-item" onSelect={() => ref && openTransfer({ kind: "export", connectionId: ref.connectionId, database: ref.database, source: { kind: "table", table: ref.name }, label: ref.name })}>Export…</DropdownMenu.Item>
                <DropdownMenu.Item className="menu-item" disabled={!isTable} onSelect={() => ref && openTransfer({ kind: "import", connectionId: ref.connectionId, database: ref.database, table: ref.name })}>Import CSV…</DropdownMenu.Item>
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
