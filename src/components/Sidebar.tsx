import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Plus, Loader2, Star } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api } from "../api/client";
import type { ConnectionConfig, Engine, ObjectInfo, ObjectKind } from "../api/types";
import { DatabaseIcon, EngineIcon, TableIcon, ViewIcon } from "./icons";
import { useConnections } from "../store/connections";
import { useWorkspace } from "../store/workspace";
import { closeConnection, openConnection } from "../lib/connect";
import { confirmAction } from "../store/confirm";
import { notify, notifyError } from "../store/toasts";
import { useDialogs } from "../store/dialogs";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { openTransfer } from "../store/transfer";
import { pasteTable, useTableClipboard } from "../store/tableClipboard";
import { MOD } from "../lib/platform";
import { Menu, type MenuItem } from "./Menu";
import { ObjectContextMenu } from "./ObjectContextMenu";
import { TreeFooter } from "./TreeFooter";
import { treeNodeKey } from "../lib/treeNodes";
import { useTreeFilter } from "../store/treeFilter";
import { isFavorite, useFavorites } from "../store/favorites";
import { createTreeClickNavigation } from "../lib/treeClickNavigation";

/** Selecting a database or object in the tree shows it in the Objects tab. */
const showObjects = () => useTabs.getState().activate(OBJECTS_TAB);

export function Sidebar() {
  const showDialog = useConnections((s) => s.showDialog);
  const connections = useQuery({ queryKey: ["connections"], queryFn: api.listConnections });
  const open = useConnections((s) => s.open);
  const activeOnly = useTreeFilter((s) => s.activeOnly);
  const engines = useTreeFilter((s) => s.engines);
  const favoritesOnly = useTreeFilter((s) => s.favoritesOnly);
  const favorites = useFavorites((s) => s.favorites);
  const [objectClickNavigation] = useState(() => createTreeClickNavigation(showObjects));
  useEffect(() => {
    const cancel = objectClickNavigation.cancel;
    document.addEventListener("pointerdown", cancel, true);
    document.addEventListener("keydown", cancel, true);
    const unsubscribe = useTabs.subscribe((state, previous) => {
      if (state.active !== previous.active) cancel();
    });
    return () => {
      document.removeEventListener("pointerdown", cancel, true);
      document.removeEventListener("keydown", cancel, true);
      unsubscribe();
      cancel();
    };
  }, [objectClickNavigation]);
  const list = connections.data ?? [];
  const shown = list.filter(
    (c) =>
      (!activeOnly || c.id in open) &&
      (engines.length === 0 || engines.includes(c.engine)) &&
      (!favoritesOnly || favorites.some((f) => f.connectionId === c.id)),
  );
  return (
    <>
      <div className="panel-header">
        <span className="grow">Connections</span>
        <button className="icon-btn" title="New connection" onClick={() => showDialog({ mode: "create" })}><Plus size={16} /></button>
      </div>
      <div
        className="tree"
        role="tree"
        tabIndex={0}
        onClick={(event) => {
          if (event.detail > 1 || !(event.target instanceof Element) || !event.target.closest("[data-tree-object]")) return;
          void objectClickNavigation.singleClick(api.doubleClickInterval()).catch((error) => notifyError("Could not open Objects", error));
        }}
        onDoubleClickCapture={objectClickNavigation.cancel}
      >
        {shown.map((c) => <ConnectionNode key={c.id} config={c} />)}
        {connections.isSuccess && list.length === 0 && (
          <div className="empty"><p>No connections yet. Create one to get started.</p></div>
        )}
        {list.length > 0 && shown.length === 0 && (
          <div className="empty"><p>{favoritesOnly ? "No favorites match the filter." : "No connections match the filter."}</p></div>
        )}
      </div>
      <TreeFooter connections={list} />
    </>
  );
}

function Row(props: {
  nodeKey: string;
  depth: number;
  expandable: boolean;
  expanded?: boolean;
  loading?: boolean;
  selected?: boolean;
  icon: ReactNode;
  label: ReactNode;
  trailing?: ReactNode;
  onToggle?: () => void;
  onClick?: () => void;
  onDoubleClick?: () => void;
}) {
  return (
    <div
      className={`tree-row${props.selected ? " selected" : ""}`}
      role="treeitem"
      data-node={props.nodeKey}
      aria-expanded={props.expandable ? !!props.expanded : undefined}
      aria-selected={!!props.selected}
      style={{ paddingLeft: 6 + props.depth * 16 }}
      onClick={() => (props.onClick ? props.onClick() : props.onToggle?.())}
      onDoubleClick={props.onDoubleClick}
    >
      <span
        className={`chevron${props.expanded ? " open" : ""}`}
        onClick={(e) => {
          if (!props.expandable) return;
          e.stopPropagation();
          props.onToggle?.();
        }}
      >
        {props.loading ? <Loader2 size={13} className="spin" /> : props.expandable && <ChevronRight size={14} />}
      </span>
      {props.icon}
      <span className="label">{props.label}</span>
      {props.trailing}
    </div>
  );
}

function ConnectionNode({ config }: { config: ConnectionConfig }) {
  const qc = useQueryClient();
  const isOpen = useConnections((s) => config.id in s.open);
  const showDialog = useConnections((s) => s.showDialog);
  const key = treeNodeKey.connection(config.id);
  const expanded = useWorkspace((s) => !!s.expanded[key]) && isOpen;
  const { setExpanded, select } = useWorkspace();
  const selected = useWorkspace((s) => s.selection?.connectionId === config.id && !s.selection.database);
  const search = useTreeFilter((s) => s.search.trim().toLowerCase());
  const favoritesOnly = useTreeFilter((s) => s.favoritesOnly);
  const favorites = useFavorites((s) => s.favorites);

  const databases = useQuery({
    queryKey: ["databases", config.id],
    queryFn: () => api.listDatabases(config.id),
    enabled: isOpen && expanded,
  });
  const shownDatabases = databases.data?.filter(
    (db) =>
      (!search || db.name.toLowerCase().includes(search)) &&
      (!favoritesOnly || favorites.some((f) => f.connectionId === config.id && f.database === db.name)),
  );

  const toggle = async () => {
    if (expanded) return setExpanded(key, false);
    if (!isOpen && !(await openConnection(config))) return;
    setExpanded(key, true);
  };

  const remove = async () => {
    const ok = await confirmAction({
      title: "Delete connection",
      message: `Delete "${config.name}"? Its saved password is removed too. Databases are not affected.`,
      confirmLabel: "Delete",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteConnection(config.id);
      useConnections.getState().markClosed(config.id);
      await qc.invalidateQueries({ queryKey: ["connections"] });
    } catch (e) {
      notifyError("Could not delete connection", e);
      // The connection may be gone even though its saved password could not be removed: show the real list.
      useConnections.getState().markClosed(config.id);
      await qc.invalidateQueries({ queryKey: ["connections"] });
    }
  };

  const items: MenuItem[] = [
    isOpen
      ? { label: "Close Connection", onSelect: () => void closeConnection(config.id, qc).then(() => setExpanded(key, false)) }
      : { label: "Open Connection", onSelect: () => void toggle() },
    { label: "Refresh", disabled: !isOpen, onSelect: () => void qc.invalidateQueries({ queryKey: ["databases", config.id] }) },
    { label: "Restore Backup…", disabled: !isOpen, onSelect: () => openTransfer({ kind: "restore", connectionId: config.id, database: null }) },
    { label: "New Query", disabled: !isOpen, onSelect: () => useTabs.getState().newQuery({ connectionId: config.id, database: config.engine === "sqlite" ? "main" : config.database }) },
    config.engine === "sqlite"
      ? { label: "Attach Database…", disabled: !isOpen, onSelect: () => useDialogs.getState().set({ attachDatabaseFor: config.id }) }
      : { label: "New Database…", disabled: !isOpen, onSelect: () => useDialogs.getState().set({ createDatabaseFor: config.id }) },
    "separator",
    { label: "Edit Connection…", onSelect: () => showDialog({ mode: "edit", config }) },
    {
      label: "Duplicate Connection",
      onSelect: () =>
        void api
          .duplicateConnection(config.id)
          .then(() => qc.invalidateQueries({ queryKey: ["connections"] }))
          .catch((e) => notifyError("Could not duplicate connection", e)),
    },
    { label: "Delete Connection…", danger: true, onSelect: () => void remove() },
  ];

  return (
    <>
      <Menu items={items}>
        <div>
          <Row
            nodeKey={key}
            depth={0}
            expandable
            expanded={expanded}
            loading={databases.isFetching && !databases.data}
            selected={selected}
            icon={<EngineIcon engine={config.engine} size={18} />}
            label={config.name}
            trailing={
              <>
                {config.color && <span className="color-tag" style={{ background: config.color }} />}
                {isOpen && <span className="status-dot on" title="Connected" />}
              </>
            }
            onToggle={toggle}
            onClick={() => select({ connectionId: config.id })}
            onDoubleClick={toggle}
          />
        </div>
      </Menu>
      {expanded && shownDatabases?.map((db) => <DatabaseNode key={db.name} connectionId={config.id} engine={config.engine} database={db.name} />)}
      {expanded && search && shownDatabases?.length === 0 && <div className="tree-empty">No databases match “{search}”</div>}
    </>
  );
}

function DatabaseNode({ connectionId, engine, database }: { connectionId: string; engine: Engine; database: string }) {
  const qc = useQueryClient();
  const key = treeNodeKey.database(connectionId, database);
  const expanded = useWorkspace((s) => !!s.expanded[key]);
  const { setExpanded, select } = useWorkspace();
  const selected = useWorkspace(
    (s) => s.selection?.connectionId === connectionId && s.selection.database === database && !s.selection.object,
  );
  const objects = useQuery({
    queryKey: ["objects", connectionId, database],
    queryFn: () => api.listObjects(connectionId, database),
    enabled: expanded,
  });
  const copied = useTableClipboard((s) => s.copied);
  const pasteLabel = copied ? `Paste ${copied.name} (${MOD}+V)` : "Paste Table";
  const toggle = () => setExpanded(key, !expanded);
  const drop = async () => {
    const sqlite = engine === "sqlite";
    const ok = await confirmAction(
      sqlite
        ? { title: "Detach database", message: `Detach "${database}" from this connection? The file is not deleted.`, confirmLabel: "Detach" }
        : {
            title: "Drop database",
            message: `Drop "${database}" and everything in it? This cannot be undone.`,
            confirmLabel: "Drop Database",
            danger: true,
            requireText: database,
          },
    );
    if (!ok) return;
    try {
      await api.dropDatabase(connectionId, database);
      const ws = useWorkspace.getState();
      if (ws.selection?.connectionId === connectionId && ws.selection.database === database) ws.select({ connectionId });
      ws.setExpanded(key, false);
      await qc.invalidateQueries({ queryKey: ["databases", connectionId] });
      notify(sqlite ? "Database detached" : "Database dropped", database);
    } catch (e) {
      notifyError(`Could not drop ${database}`, e);
    }
  };
  const items: MenuItem[] = [
    { label: "New Query", onSelect: () => useTabs.getState().newQuery({ connectionId, database }) },
    { label: "New Table…", onSelect: () => useTabs.getState().newTable(connectionId, database) },
    { label: "Run SQL File…", onSelect: () => openTransfer({ kind: "runSql", connectionId, database }) },
    { label: "Backup…", onSelect: () => openTransfer({ kind: "backup", connectionId, database, data: true, perTable: false }) },
    { label: "Restore Backup…", onSelect: () => openTransfer({ kind: "restore", connectionId, database }) },
    { label: "Refresh", onSelect: () => void qc.invalidateQueries({ queryKey: ["objects", connectionId, database] }) },
    { label: pasteLabel, disabled: !copied, onSelect: () => pasteTable({ connectionId, database }) },
    "separator",
    engine === "sqlite"
      ? { label: "Detach Database", disabled: database === "main", onSelect: () => void drop() }
      : { label: "Drop Database…", danger: true, onSelect: () => void drop() },
  ];
  return (
    <>
      <Menu items={items}>
        <div>
          <Row
            nodeKey={key}
            depth={1}
            expandable
            expanded={expanded}
            loading={objects.isFetching && !objects.data}
            selected={selected}
            icon={<DatabaseIcon size={16} />}
            label={database}
            onToggle={toggle}
            onClick={() => { select({ connectionId, database }); showObjects(); }}
            onDoubleClick={toggle}
          />
        </div>
      </Menu>
      {expanded && objects.data && (
        <>
          <GroupNode connectionId={connectionId} database={database} kind="table" objects={objects.data} />
          <GroupNode connectionId={connectionId} database={database} kind="view" objects={objects.data} />
        </>
      )}
    </>
  );
}

function GroupNode(props: { connectionId: string; database: string; kind: ObjectKind; objects: ObjectInfo[] }) {
  const { connectionId, database, kind } = props;
  const key = treeNodeKey.group(connectionId, database, kind);
  const favoritesOnly = useTreeFilter((s) => s.favoritesOnly);
  const favorites = useFavorites((s) => s.favorites);
  const expandedFlag = useWorkspace((s) => !!s.expanded[key]);
  const setExpanded = useWorkspace((s) => s.setExpanded);
  const copied = useTableClipboard((s) => s.copied);
  const all = props.objects.filter((o) => o.kind === kind);
  // The toolbar's "Search objects" filters only the object list; the tree is filtered by its footer and favorites.
  const shown = favoritesOnly ? all.filter((o) => isFavorite(favorites, { connectionId, database, name: o.name })) : all;
  const expanded = expandedFlag || (favoritesOnly && shown.length > 0);
  if (favoritesOnly && shown.length === 0) return null;
  const Icon = kind === "table" ? TableIcon : ViewIcon;
  const groupItems: MenuItem[] =
    kind === "table"
      ? [
          { label: "New Table…", onSelect: () => useTabs.getState().newTable(connectionId, database) },
          { label: copied ? `Paste ${copied.name} (${MOD}+V)` : "Paste Table", disabled: !copied, onSelect: () => pasteTable({ connectionId, database }) },
        ]
      : [];
  return (
    <>
      <Menu items={groupItems}>
      <div>
      <Row
        nodeKey={key}
        depth={2}
        expandable
        expanded={expanded}
        icon={<Icon size={16} />}
        label={kind === "table" ? "Tables" : "Views"}
        trailing={<span className="count">{favoritesOnly ? `${shown.length}/${all.length}` : all.length}</span>}
        onToggle={() => setExpanded(key, !expandedFlag)}
        onClick={() => {
          useWorkspace.getState().select({ connectionId, database });
          showObjects();
        }}
      />
      </div>
      </Menu>
      {expanded &&
        shown.map((o) => <ObjectNode key={o.name} connectionId={connectionId} database={database} object={o} />)}
    </>
  );
}

function ObjectNode({ connectionId, database, object }: { connectionId: string; database: string; object: ObjectInfo }) {
  const select = useWorkspace((s) => s.select);
  const selected = useWorkspace(
    (s) =>
      s.selection?.connectionId === connectionId &&
      s.selection.database === database &&
      s.selection.object?.name === object.name,
  );
  const favorite = useFavorites((s) => isFavorite(s.favorites, { connectionId, database, name: object.name }));
  const Icon = object.kind === "table" ? TableIcon : ViewIcon;
  const open = () => useTabs.getState().openTable({ connectionId, database, name: object.name, objectKind: object.kind }, "data");
  return (
    <ObjectContextMenu object={{ connectionId, database, name: object.name, kind: object.kind }}>
      <div data-tree-object>
        <Row
          nodeKey={treeNodeKey.object(connectionId, database, object.name)}
          depth={3}
          expandable={false}
          selected={selected}
          icon={<Icon size={16} />}
          label={object.name}
          trailing={favorite && <Star size={11} className="fav-star" aria-label="Favorite" />}
          onClick={() => select({ connectionId, database, object: { name: object.name, kind: object.kind } })}
          onDoubleClick={open}
        />
      </div>
    </ObjectContextMenu>
  );
}
