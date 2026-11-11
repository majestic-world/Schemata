import type { ObjectKind } from "../api/types";

/** Keys of schema-tree nodes: used for the persisted expanded state and as the rows' `data-node`. */
export const treeNodeKey = {
  connection: (connectionId: string) => `c:${connectionId}`,
  database: (connectionId: string, database: string) => `d:${connectionId}/${database}`,
  group: (connectionId: string, database: string, kind: ObjectKind) => `g:${connectionId}/${database}/${kind}`,
  object: (connectionId: string, database: string, name: string) => `o:${connectionId}/${database}/${name}`,
};
