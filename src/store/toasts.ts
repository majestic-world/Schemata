import { create } from "zustand";

export type Toast = { id: number; tone: "error" | "info" | "success"; title: string; message?: string };

type ToastState = {
  toasts: Toast[];
  push: (t: Omit<Toast, "id">) => void;
  dismiss: (id: number) => void;
};

let nextId = 1;

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push: (t) => {
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts, { ...t, id }] }));
    setTimeout(() => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })), t.tone === "error" ? 8000 : 4000);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),
}));

export function notifyError(title: string, e: unknown) {
  const message = typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : String(e);
  useToasts.getState().push({ tone: "error", title, message });
}

export function notify(title: string, message?: string) {
  useToasts.getState().push({ tone: "success", title, message });
}
