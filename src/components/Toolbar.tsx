import { Search, PanelRight } from "lucide-react";
import type { ReactNode } from "react";
import { useState } from "react";
import { useLayout } from "../store/layout";
import { useConnections } from "../store/connections";
import { useWorkspace } from "../store/workspace";
import { useTabs } from "../store/tabs";
import { openTransfer } from "../store/transfer";
import { MOD } from "../lib/platform";
import { BackupIcon, ConnectionIcon, ExportIcon, ImportIcon, NewQueryIcon, SettingsIcon, TableIcon, ViewIcon } from "./icons";
import { SettingsDialog } from "./SettingsDialog";

function ToolButton(props: { icon: ReactNode; label: string; title?: string; disabled?: boolean; onClick: () => void }) {
  return (
    <button className="tool-btn" disabled={props.disabled} title={props.title} onClick={props.onClick}>
      <span className="tool-icon">{props.icon}</span>
      <span className="tool-label">{props.label}</span>
    </button>
  );
}

export function Toolbar() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { inspectorOpen, toggleInspector } = useLayout();
  const showDialog = useConnections((s) => s.showDialog);
  const { search, setSearch, selection, selectedObjects } = useWorkspace();
  const db = selection?.database;
  const object = selection?.object;
  return (
    <header className="toolbar">
      <ToolButton icon={<ConnectionIcon />} label="Connection" title="New connection" onClick={() => showDialog({ mode: "create" })} />
      <ToolButton
        icon={<NewQueryIcon />}
        label="New Query"
        title={`New query (${MOD}+T)`}
        disabled={!selection}
        onClick={() => selection && useTabs.getState().newQuery({ connectionId: selection.connectionId, database: db ?? null })}
      />
      <div className="tool-sep" />
      <ToolButton
        icon={<TableIcon />}
        label="Table"
        title="New table in the selected database"
        disabled={!db}
        onClick={() => selection && db && useTabs.getState().newTable(selection.connectionId, db)}
      />
      <ToolButton
        icon={<ViewIcon />}
        label="View"
        title="New view in the selected database"
        disabled={!db}
        onClick={() =>
          selection && db && useTabs.getState().newQuery({ connectionId: selection.connectionId, database: db, title: "New view", sql: "CREATE VIEW new_view AS\nSELECT\n  *\nFROM table_name;\n" })
        }
      />
      <div className="tool-sep" />
      <ToolButton
        icon={<BackupIcon />}
        label="Backup"
        title="Back up the selected database"
        disabled={!db}
        onClick={() => selection && db && openTransfer({ kind: "backup", connectionId: selection.connectionId, database: db, data: true })}
      />
      <ToolButton
        icon={<ImportIcon />}
        label="Import"
        title={object?.kind === "table" ? "Import a CSV file into the selected table" : "Run a SQL file on the selected database"}
        disabled={!db || selectedObjects.length > 1}
        onClick={() =>
          selection && db && (object?.kind === "table"
            ? openTransfer({ kind: "import", connectionId: selection.connectionId, database: db, table: object.name })
            : openTransfer({ kind: "runSql", connectionId: selection.connectionId, database: db }))
        }
      />
      <ToolButton
        icon={<ExportIcon />}
        label="Export"
        title="Export the selected table or view"
        disabled={!db || !object || selectedObjects.length > 1}
        onClick={() => selection && db && object && openTransfer({ kind: "export", connectionId: selection.connectionId, database: db, source: { kind: "table", table: object.name }, label: object.name })}
      />
      <ToolButton icon={<SettingsIcon />} label="Settings" title="Settings" onClick={() => setSettingsOpen(true)} />
      <div className="toolbar-spacer" />
      <label className="search">
        <Search size={14} />
        <input id="global-search" placeholder="Search objects" value={search} onChange={(e) => setSearch(e.target.value)} />
        <kbd>{MOD} K</kbd>
      </label>
      <button className={`icon-btn toggle${inspectorOpen ? " on" : ""}`} title="Show or hide the inspector" aria-pressed={inspectorOpen} onClick={toggleInspector}>
        <PanelRight size={18} />
      </button>
      <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </header>
  );
}
