import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ContextMenu } from "radix-ui";
import { ChevronRight, Plus, Database as DbIcon, Table2, Eye, Loader2 } from "lucide-react";
import type { ReactNode } from "react";
import { api } from "../api/client";
import type { ConnectionConfig, ObjectInfo, ObjectKind } from "../api/types";
import { EngineIcon } from "./EngineIcon";
import { useConnections } from "../store/connections";
import { useWorkspace } from "../store/workspace";
import { closeConnection, openConnection } from "../lib/connect";
import { confirmAction } from "../store/confirm";
import { notifyError } from "../store/toasts";

export function Sidebar() {
  const showDialog = useConnections((s) => s.showDialog);
  const connections = useQuery({ queryKey: ["connections"], queryFn: api.listConnections });
  const list = connections.data ?? [];
  return (
    <>
      <div className="panel-header">
        <span className="grow">Connections</span>
        <button className="icon-btn" title="New connection" onClick={() => showDialog({ mode: "create" })}><Plus size={16} /></button>
      </div>
      <div className="tree" role="tree">
        {list.map((c) => <ConnectionNode key={c.id} config={c} />)}
        {connections.isSuccess && list.length === 0 && (
          <div className="empty"><p>No connections yet. Create one to get started.</p></div>
        )}
      </div>
    </>
  );
}

type Item = { label: string; onSelect: () => void; danger?: boolean; disabled?: boolean } | "separator";

export function Menu({ items, children }: { items: Item[]; children: ReactNode }) {
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild>{children}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="menu">
          {items.map((it, i) =>
            it === "separator" ? (
              <ContextMenu.Separator key={i} className="menu-sep" />
            ) : (
              <ContextMenu.Item key={it.label} className={`menu-item${it.danger ? " danger" : ""}`} disabled={it.disabled} onSelect={it.onSelect}>
                {it.label}
              </ContextMenu.Item>
            ),
          )}
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

function Row(props: {
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
      aria-expanded={props.expandable ? !!props.expanded : undefined}
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
  const key = `c:${config.id}`;
  const expanded = useWorkspace((s) => !!s.expanded[key]) && isOpen;
  const { setExpanded, select } = useWorkspace();
  const selected = useWorkspace((s) => s.selection?.connectionId === config.id && !s.selection.database);

  const databases = useQuery({
    queryKey: ["databases", config.id],
    queryFn: () => api.listDatabases(config.id),
    enabled: isOpen && expanded,
  });

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
    }
  };

  const items: Item[] = [
    isOpen
      ? { label: "Close Connection", onSelect: () => void closeConnection(config.id, qc).then(() => setExpanded(key, false)) }
      : { label: "Open Connection", onSelect: () => void toggle() },
    { label: "Refresh", disabled: !isOpen, onSelect: () => void qc.invalidateQueries({ queryKey: ["databases", config.id] }) },
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
            depth={0}
            expandable
            expanded={expanded}
            loading={databases.isFetching && !databases.data}
            selected={selected}
            icon={<EngineIcon engine={config.engine} />}
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
      {expanded &&
        databases.data?.map((db) => <DatabaseNode key={db.name} connectionId={config.id} database={db.name} />)}
    </>
  );
}

function DatabaseNode({ connectionId, database }: { connectionId: string; database: string }) {
  const qc = useQueryClient();
  const key = `d:${connectionId}/${database}`;
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
  const toggle = () => setExpanded(key, !expanded);
  const items: Item[] = [
    { label: "Refresh", onSelect: () => void qc.invalidateQueries({ queryKey: ["objects", connectionId, database] }) },
  ];
  return (
    <>
      <Menu items={items}>
        <div>
          <Row
            depth={1}
            expandable
            expanded={expanded}
            loading={objects.isFetching && !objects.data}
            selected={selected}
            icon={<DbIcon size={16} className="tree-icon db" />}
            label={database}
            onToggle={toggle}
            onClick={() => select({ connectionId, database })}
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
  const key = `g:${connectionId}/${database}/${kind}`;
  const search = useWorkspace((s) => s.search.trim().toLowerCase());
  const expandedFlag = useWorkspace((s) => !!s.expanded[key]);
  const setExpanded = useWorkspace((s) => s.setExpanded);
  const all = props.objects.filter((o) => o.kind === kind);
  const shown = search ? all.filter((o) => o.name.toLowerCase().includes(search)) : all;
  const expanded = expandedFlag || (search !== "" && shown.length > 0);
  if (search && shown.length === 0) return null;
  const Icon = kind === "table" ? Table2 : Eye;
  return (
    <>
      <Row
        depth={2}
        expandable
        expanded={expanded}
        icon={<Icon size={16} className="tree-icon" />}
        label={kind === "table" ? "Tables" : "Views"}
        trailing={<span className="count">{search ? `${shown.length}/${all.length}` : all.length}</span>}
        onToggle={() => setExpanded(key, !expandedFlag)}
      />
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
  const Icon = object.kind === "table" ? Table2 : Eye;
  return (
    <Row
      depth={3}
      expandable={false}
      selected={selected}
      icon={<Icon size={15} className="tree-icon obj" />}
      label={object.name}
      onClick={() => select({ connectionId, database, object: { name: object.name, kind: object.kind } })}
    />
  );
}
