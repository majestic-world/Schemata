import type { ObjectKind } from "../api/types";

/** Keys of schema-tree nodes: used for the persisted expanded state and as the rows' `data-node`. */
export const treeNodeKey = {
  connection: (connectionId: string) => `c:${connectionId}`,
  database: (connectionId: string, database: string) => `d:${connectionId}/${database}`,
  group: (connectionId: string, database: string, kind: ObjectKind) => `g:${connectionId}/${database}/${kind}`,
  object: (connectionId: string, database: string, name: string) => `o:${connectionId}/${database}/${name}`,
  /** Saved queries of a database, or of the connection when they run without one. */
  queries: (connectionId: string, database: string | null) => `q:${connectionId}/${database ?? ""}`,
  savedQuery: (id: string) => `sq:${id}`,
};
