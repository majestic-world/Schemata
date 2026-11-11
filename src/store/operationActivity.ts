import { create } from "zustand";

type OperationActivity = {
  active: ReadonlySet<string>;
  begin: (id: string) => void;
  end: (id: string) => void;
};

/** Transfer operations outlive their dialog's render state while their native call is running. */
export const useOperationActivity = create<OperationActivity>((set) => ({
  active: new Set(),
  begin: (id) => set((s) => ({ active: new Set([...s.active, id]) })),
  end: (id) => set((s) => {
    const active = new Set(s.active);
    active.delete(id);
    return { active };
  }),
}));
