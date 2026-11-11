import { create } from "zustand";

export type ConfirmRequest = {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  /** When set, the user must type this text to enable the confirm button. */
  requireText?: string;
  /** When set, the dialog asks for a value, prefilled with this text. */
  input?: { label: string; initial: string };
};

type Pending = ConfirmRequest & { resolve: (value: string | null) => void };

type ConfirmState = {
  request: Pending | null;
  ask: (req: ConfirmRequest) => Promise<string | null>;
  settle: (value: string | null) => void;
};

export const useConfirm = create<ConfirmState>((set, get) => ({
  request: null,
  ask: (req) => new Promise((resolve) => set({ request: { ...req, resolve } })),
  settle: (value) => {
    get().request?.resolve(value);
    set({ request: null });
  },
}));

export const confirmAction = async (req: ConfirmRequest) => (await useConfirm.getState().ask(req)) !== null;
/** Asks for a value; resolves to `null` when cancelled. */
export const promptAction = (req: ConfirmRequest & { input: { label: string; initial: string } }) => useConfirm.getState().ask(req);
