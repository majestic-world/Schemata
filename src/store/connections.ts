import { create } from "zustand";
import type { ConnectionConfig } from "../api/types";

type DialogState = { mode: "create" } | { mode: "edit"; config: ConnectionConfig };

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
