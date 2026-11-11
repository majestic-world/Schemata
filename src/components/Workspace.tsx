import { X, Table2, Eye, LayoutGrid } from "lucide-react";
import { ObjectList } from "./ObjectList";
import { TableTabView } from "./TableTabView";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { useWorkspace } from "../store/workspace";

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
          const Icon = t.objectKind === "table" ? Table2 : Eye;
          return (
            <div
              key={t.id}
              className={`tab${active === t.id ? " active" : ""}`}
              role="tab"
              title={`${t.name}@${t.database}`}
              onClick={() => {
                activate(t.id);
                useWorkspace.getState().select({ connectionId: t.connectionId, database: t.database, object: { name: t.name, kind: t.objectKind } });
              }}
              onAuxClick={(e) => e.button === 1 && close(t.id)}
            >
              <Icon size={14} className="tree-icon obj" />
              <span className="title">{t.name}@{t.database}</span>
              <button className="icon-btn tab-close" aria-label="Close tab" onClick={(e) => { e.stopPropagation(); close(t.id); }}>
                <X size={13} />
              </button>
            </div>
          );
        })}
      </div>
      <div className="tab-body">
        {activeTab ? <TableTabView key={activeTab.id} tab={activeTab} /> : <ObjectsView />}
      </div>
    </>
  );
}

function ObjectsView() {
  const selection = useWorkspace((s) => s.selection);
  const openTable = useTabs((s) => s.openTable);
  const obj = selection?.object;
  const open = (view: "data" | "structure") =>
    selection?.database && obj && openTable({ connectionId: selection.connectionId, database: selection.database, name: obj.name, objectKind: obj.kind }, view);
  return (
    <ObjectList
      onOpen={(o) =>
        selection?.database && openTable({ connectionId: selection.connectionId, database: selection.database, name: o.name, objectKind: o.kind })
      }
      actions={
        <>
          <button className="btn primary" disabled={!obj} onClick={() => open("data")}>
            {obj?.kind === "view" ? <Eye size={15} /> : <Table2 size={15} />}Open {obj?.kind === "view" ? "View" : "Table"}
          </button>
        </>
      }
    />
  );
}
