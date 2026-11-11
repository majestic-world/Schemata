import { Plug, FilePlus2, Table2, Eye, DatabaseBackup, Download, Upload, Search, PanelRight } from "lucide-react";
import logo from "../assets/logo.svg";
import { useLayout } from "../store/layout";
import { useConnections } from "../store/connections";
import { useWorkspace } from "../store/workspace";
import { useTabs } from "../store/tabs";
import { openTransfer } from "../store/transfer";
import { MOD } from "../lib/platform";

export function Toolbar() {
  const toggleInspector = useLayout((s) => s.toggleInspector);
  const showDialog = useConnections((s) => s.showDialog);
  const { search, setSearch, selection } = useWorkspace();
  const db = selection?.database;
  return (
    <header className="toolbar">
      <div className="brand">
        <img src={logo} alt="" />
        <span>ProjectCat</span>
      </div>
      <button className="tool-btn primary" onClick={() => showDialog({ mode: "create" })}><Plug size={20} />Connection</button>
      <button className="tool-btn" disabled={!selection} title={`New query (${MOD}+T)`} onClick={() => selection && useTabs.getState().newQuery({ connectionId: selection.connectionId, database: db ?? null })}><FilePlus2 size={20} />New Query</button>
      <button className="tool-btn" disabled={!db} title="New table in the selected database" onClick={() => selection && db && useTabs.getState().newTable(selection.connectionId, db)}><Table2 size={20} />Table</button>
      <button className="tool-btn" disabled={!db} title="New view in the selected database" onClick={() => selection && db && useTabs.getState().newQuery({ connectionId: selection.connectionId, database: db, title: "New view", sql: "CREATE VIEW new_view AS\nSELECT\n  *\nFROM table_name;\n" })}><Eye size={20} />View</button>
      <button className="tool-btn" disabled={!db} title="Back up the selected database" onClick={() => selection && db && openTransfer({ kind: "backup", connectionId: selection.connectionId, database: db })}><DatabaseBackup size={20} />Backup</button>
      <button
        className="tool-btn"
        disabled={!db}
        title={selection?.object?.kind === "table" ? "Import a CSV file into the selected table" : "Run a SQL file on the selected database"}
        onClick={() =>
          selection && db && (selection.object?.kind === "table"
            ? openTransfer({ kind: "import", connectionId: selection.connectionId, database: db, table: selection.object.name })
            : openTransfer({ kind: "runSql", connectionId: selection.connectionId, database: db }))
        }
      ><Download size={20} />Import</button>
      <button
        className="tool-btn"
        disabled={!db || !selection?.object}
        title="Export the selected table or view"
        onClick={() => selection && db && selection.object && openTransfer({ kind: "export", connectionId: selection.connectionId, database: db, source: { kind: "table", table: selection.object.name }, label: selection.object.name })}
      ><Upload size={20} />Export</button>
      <div className="toolbar-spacer" />
      <label className="search">
        <Search size={15} />
        <input id="global-search" placeholder="Search objects" value={search} onChange={(e) => setSearch(e.target.value)} />
        <kbd>{MOD} K</kbd>
      </label>
      <button className="icon-btn" title="Toggle inspector" onClick={toggleInspector}><PanelRight size={18} /></button>
    </header>
  );
}
