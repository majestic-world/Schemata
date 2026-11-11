import { create } from "zustand";

type DialogsState = {
  createDatabaseFor: string | null;
  attachDatabaseFor: string | null;
  set: (p: Partial<Pick<DialogsState, "createDatabaseFor" | "attachDatabaseFor">>) => void;
};

export const useDialogs = create<DialogsState>((set) => ({
  createDatabaseFor: null,
  attachDatabaseFor: null,
  set: (p) => set(p),
}));
