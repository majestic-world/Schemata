import { Popover } from "radix-ui";
import { ChevronsDownUp, ListFilter, LocateFixed, Search, Star, X } from "lucide-react";
import { ENGINE_LABEL, type ConnectionConfig, type Engine } from "../api/types";
import { openConnection } from "../lib/connect";
import { treeNodeKey } from "../lib/treeNodes";
import { useConnections } from "../store/connections";
import { isFavorite, useFavorites } from "../store/favorites";
import { useTabs } from "../store/tabs";
import { useTreeFilter } from "../store/treeFilter";
import { useWorkspace, type Selection } from "../store/workspace";
import { EngineIcon } from "./icons";

const ENGINES = Object.keys(ENGINE_LABEL) as Engine[];

/** Navicat-style tree footer: database search, connection filter, favorites, locate and collapse all. */
export function TreeFooter({ connections }: { connections: ConnectionConfig[] }) {
  const { search, setSearch, activeOnly, setActiveOnly, engines, toggleEngine, clearFilters, favoritesOnly, setFavoritesOnly } = useTreeFilter();
  const collapseAll = useWorkspace((s) => s.collapseAll);
  const filtered = activeOnly || engines.length > 0;
  return (
    <div className="tree-footer">
      <label className="tree-search">
        <Search size={13} />
        <input
          placeholder="Search"
          aria-label="Search databases"
          spellCheck={false}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setSearch("")}
        />
        {search && <button className="icon-btn" aria-label="Clear search" onClick={() => setSearch("")}><X size={12} /></button>}
      </label>
      <Popover.Root>
        <Popover.Trigger asChild>
          <button className={`icon-btn${filtered ? " on" : ""}`} title="Filter" aria-label="Filter connections"><ListFilter size={15} /></button>
        </Popover.Trigger>
        <Popover.Portal>
          <Popover.Content className="tree-filter" side="top" align="start" sideOffset={6} collisionPadding={8}>
            <label className="check" title="Show only open connections">
              <input type="checkbox" checked={activeOnly} onChange={(e) => setActiveOnly(e.target.checked)} />
              Active Object
            </label>
            <div className="menu-sep" />
            <div className="tree-filter-title">Connection Filter</div>
            {ENGINES.map((engine) => (
              <label key={engine} className="check">
                <input type="checkbox" checked={engines.includes(engine)} onChange={() => toggleEngine(engine)} />
                <EngineIcon engine={engine} size={16} />
                {ENGINE_LABEL[engine]}
              </label>
            ))}
            <div className="menu-sep" />
            <button className="tree-filter-clear" disabled={!filtered} onClick={clearFilters}>Clear All</button>
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>
      <button
        className={`icon-btn${favoritesOnly ? " on" : ""}`}
        title="Show favorites only"
        aria-label="Show favorites only"
        aria-pressed={favoritesOnly}
        onClick={() => setFavoritesOnly(!favoritesOnly)}
      >
        <Star size={15} fill={favoritesOnly ? "currentColor" : "none"} />
      </button>
      <button className="icon-btn" title="Locate active object" aria-label="Locate active object" onClick={() => void locateActiveObject(connections)}>
        <LocateFixed size={15} />
      </button>
      <button className="icon-btn" title="Collapse all" aria-label="Collapse all" onClick={collapseAll}><ChevronsDownUp size={15} /></button>
    </div>
  );
}

/** The object of the active tab, or the tree/object-list selection when the Objects tab is in front. */
function activeTarget(): Selection | null {
  const tabs = useTabs.getState();
  const tab = tabs.tabs.find((t) => t.id === tabs.active);
  if (!tab) return useWorkspace.getState().selection;
  return {
    connectionId: tab.connectionId,
    database: tab.database ?? undefined,
    object: tab.kind === "table" ? { name: tab.name, kind: tab.objectKind } : undefined,
  };
}

/** Expands the tree down to the active object, selects it and scrolls it into view, lifting filters that hide it. */
async function locateActiveObject(connections: ConnectionConfig[]) {
  const target = activeTarget();
  const config = target && connections.find((c) => c.id === target.connectionId);
  if (!target || !config) return;
  if (!(config.id in useConnections.getState().open) && !(await openConnection(config))) return;

  const filter = useTreeFilter.getState();
  if (filter.engines.length > 0 && !filter.engines.includes(config.engine)) filter.clearFilters();
  const search = filter.search.trim().toLowerCase();
  if (search && !target.database?.toLowerCase().includes(search)) filter.setSearch("");
  const { database, object } = target;
  if (filter.favoritesOnly && !(database && object && isFavorite(useFavorites.getState().favorites, { connectionId: config.id, database, name: object.name }))) {
    filter.setFavoritesOnly(false);
  }

  const workspace = useWorkspace.getState();
  workspace.setExpanded(treeNodeKey.connection(config.id), true);
  let key = treeNodeKey.connection(config.id);
  if (database) {
    workspace.setExpanded(treeNodeKey.database(config.id, database), true);
    key = treeNodeKey.database(config.id, database);
    if (object) {
      workspace.setExpanded(treeNodeKey.group(config.id, database, object.kind), true);
      key = treeNodeKey.object(config.id, database, object.name);
    }
  }
  workspace.select(target);
  reveal(key);
}

/** Waits for the row to render (databases and objects may still be loading), then scrolls to it. */
function reveal(key: string) {
  const deadline = performance.now() + 5000;
  const step = () => {
    const row = document.querySelector<HTMLElement>(`.tree [data-node="${CSS.escape(key)}"]`);
    if (row) {
      row.scrollIntoView({ block: "nearest" });
      row.closest<HTMLElement>(".tree")?.focus({ preventScroll: true });
    } else if (performance.now() < deadline) {
      requestAnimationFrame(step);
    }
  };
  requestAnimationFrame(step);
}
