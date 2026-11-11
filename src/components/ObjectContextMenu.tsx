import { useQueryClient } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { copyCreateStatement, dropObjects, renameTable, truncateTable, type ObjectRef } from "../lib/objectActions";
import { MOD } from "../lib/platform";
import { copyTable } from "../store/tableClipboard";
import { useTabs, type TableView } from "../store/tabs";
import { openTransfer } from "../store/transfer";
import { useWorkspace, type SelectedObject } from "../store/workspace";
import { isFavorite, useFavorites } from "../store/favorites";
import { Menu, type MenuItem } from "./Menu";

/** Shared by the schema tree and object list; actions always use the clicked object. */
export function ObjectContextMenu({ object, children }: { object: ObjectRef; children: ReactNode }) {
  const qc = useQueryClient();
  const { connectionId, database, name, kind } = object;
  const isTable = kind === "table";
  const [targets, setTargets] = useState<SelectedObject[]>([object]);
  const multiple = targets.length > 1;
  const open = (view: TableView) => useTabs.getState().openTable({ connectionId, database, name, objectKind: kind }, view);
  const favorites = useFavorites((s) => s.favorites);
  const targetFavorites = targets.map((t) => ({ connectionId, database, name: t.name }));
  const allFavorite = targetFavorites.every((f) => isFavorite(favorites, f));
  const items: MenuItem[] = [
    { label: isTable ? "Open Table" : "Open View", disabled: multiple, onSelect: () => open("data") },
    { label: isTable ? "Design Table" : "Definition", disabled: multiple, onSelect: () => open(isTable ? "structure" : "info") },
    allFavorite
      ? { label: "Remove from Favorites", onSelect: () => useFavorites.getState().remove(targetFavorites) }
      : { label: "Add to Favorites", onSelect: () => useFavorites.getState().add(targetFavorites) },
    "separator",
    ...(isTable
      ? [
          { label: `Copy (${MOD}+C)`, disabled: multiple, onSelect: () => copyTable({ connectionId, database, name }) },
          { label: "Rename… (F6)", disabled: multiple, onSelect: () => void renameTable(object, qc) },
        ]
      : []),
    { label: "Copy CREATE Statement", disabled: multiple, onSelect: () => void copyCreateStatement(object) },
    { label: "Export…", disabled: multiple, onSelect: () => openTransfer({ kind: "export", connectionId, database, source: { kind: "table", table: name }, label: name }) },
    ...(isTable
      ? [
          { label: "Import CSV…", disabled: multiple, onSelect: () => openTransfer({ kind: "import", connectionId, database, table: name }) },
          { label: "Generate Data…", disabled: multiple, onSelect: () => openTransfer({ kind: "generate", connectionId, database, table: name }) },
        ]
      : []),
    "separator",
    ...(isTable ? [{ label: "Empty Table…", disabled: multiple, danger: true, onSelect: () => void truncateTable(object, qc) }] : []),
    { label: multiple ? `Delete ${targets.length} Objects…` : isTable ? "Delete Table…" : "Delete View…", danger: true, onSelect: () => void dropObjects({ connectionId, database }, targets, qc) },
  ];

  return (
    <Menu items={items} onOpenChange={(opened) => {
      if (!opened) return;
      const workspace = useWorkspace.getState();
      if (
        workspace.selection?.connectionId === connectionId && workspace.selection.database === database &&
        workspace.selectedObjects.some((selected) => selected.name === name && selected.kind === kind)
      ) {
        setTargets(workspace.selectedObjects);
      } else {
        workspace.select({ connectionId, database, object: { name, kind } });
        setTargets([{ name, kind }]);
      }
    }}>
      {children}
    </Menu>
  );
}
