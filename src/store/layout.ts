import { create } from "zustand";
import { persist } from "zustand/middleware";

type LayoutState = {
  sidebarWidth: number;
  inspectorWidth: number;
  inspectorOpen: boolean;
  setSidebarWidth: (w: number) => void;
  setInspectorWidth: (w: number) => void;
  toggleInspector: () => void;
};

export const useLayout = create<LayoutState>()(
  persist(
    (set) => ({
      sidebarWidth: 280,
      inspectorWidth: 340,
      inspectorOpen: true,
      setSidebarWidth: (sidebarWidth) => set({ sidebarWidth }),
      setInspectorWidth: (inspectorWidth) => set({ inspectorWidth }),
      toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
    }),
    { name: "projectcat.layout" },
  ),
);
