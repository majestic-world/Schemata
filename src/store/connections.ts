import { create } from "zustand";
import type { ConnectionConfig } from "../api/types";

type DialogState = { mode: "create" } | { mode: "edit"; config: ConnectionConfig };

/** Cosmetic and credential changes do not change the live session's target. */
export function connectionTargetChanged(original: ConnectionConfig, next: ConnectionConfig): boolean {
  return original.engine !== next.engine
    || original.host !== next.host
    || original.port !== next.port
    || original.user !== next.user
    || original.database !== next.database
    || original.filePath !== next.filePath;
}

type ConnectionsState = {
  /** Server version per open connection id. */
  open: Record<string, string>;
  dialog: DialogState | null;
  /** Connection waiting for a password before it can open. */
  passwordFor: ConnectionConfig | null;
  markOpen: (id: string, version: string) => void;
  markClosed: (id: string) => void;
  showDialog: (d: DialogState | null) => void;
  askPassword: (c: ConnectionConfig | null) => void;
};

export const useConnections = create<ConnectionsState>((set) => ({
  open: {},
  dialog: null,
  passwordFor: null,
  markOpen: (id, version) => set((s) => ({ open: { ...s.open, [id]: version } })),
  markClosed: (id) =>
    set((s) => {
      const open = { ...s.open };
      delete open[id];
      return { open };
    }),
  showDialog: (dialog) => set({ dialog }),
  askPassword: (passwordFor) => set({ passwordFor }),
}));
