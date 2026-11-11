import { create } from "zustand";

export type ConfirmRequest = {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  /** When set, the user must type this text to enable the confirm button. */
  requireText?: string;
};

type ConfirmState = {
  request: (ConfirmRequest & { resolve: (ok: boolean) => void }) | null;
  ask: (req: ConfirmRequest) => Promise<boolean>;
  settle: (ok: boolean) => void;
};

export const useConfirm = create<ConfirmState>((set, get) => ({
  request: null,
  ask: (req) => new Promise<boolean>((resolve) => set({ request: { ...req, resolve } })),
  settle: (ok) => {
    get().request?.resolve(ok);
    set({ request: null });
  },
}));

export const confirmAction = (req: ConfirmRequest) => useConfirm.getState().ask(req);
