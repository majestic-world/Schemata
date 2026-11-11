import { Plug, FilePlus2, Table2, Eye, DatabaseBackup, Download, Upload, Search, PanelRight } from "lucide-react";
import logo from "../assets/logo.svg";
import { useLayout } from "../store/layout";

export function Toolbar() {
  const toggleInspector = useLayout((s) => s.toggleInspector);
  return (
    <header className="toolbar">
      <div className="brand">
        <img src={logo} alt="" />
        <span>ProjectCat</span>
      </div>
      <button className="tool-btn primary"><Plug size={20} />Connection</button>
      <button className="tool-btn" disabled><FilePlus2 size={20} />New Query</button>
      <button className="tool-btn" disabled><Table2 size={20} />Table</button>
      <button className="tool-btn" disabled><Eye size={20} />View</button>
      <button className="tool-btn" disabled><DatabaseBackup size={20} />Backup</button>
      <button className="tool-btn" disabled><Download size={20} />Import</button>
      <button className="tool-btn" disabled><Upload size={20} />Export</button>
      <div className="toolbar-spacer" />
      <label className="search">
        <Search size={15} />
        <input placeholder="Search objects" />
        <kbd>Ctrl K</kbd>
      </label>
      <button className="icon-btn" title="Toggle inspector" onClick={toggleInspector}><PanelRight size={18} /></button>
    </header>
  );
}
