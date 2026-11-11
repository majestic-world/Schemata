import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Engine } from "../api/types";

/** What the schema tree shows: the footer search, the filter popover and the favorites toggle. */
type TreeFilterState = {
  /** Database-name search; transient, not persisted. */
  search: string;
  /** Only connections that are currently open. */
  activeOnly: boolean;
  /** Engines to show; empty shows every engine. */
  engines: Engine[];
  /** Only connections, databases and objects that contain or are favorites. */
  favoritesOnly: boolean;
  setSearch: (search: string) => void;
  setActiveOnly: (activeOnly: boolean) => void;
  toggleEngine: (engine: Engine) => void;
  clearFilters: () => void;
  setFavoritesOnly: (favoritesOnly: boolean) => void;
};

export const useTreeFilter = create<TreeFilterState>()(
  persist(
    (set) => ({
      search: "",
      activeOnly: false,
      engines: [],
      favoritesOnly: false,
      setSearch: (search) => set({ search }),
      setActiveOnly: (activeOnly) => set({ activeOnly }),
      toggleEngine: (engine) => set((s) => ({ engines: s.engines.includes(engine) ? s.engines.filter((e) => e !== engine) : [...s.engines, engine] })),
      clearFilters: () => set({ activeOnly: false, engines: [] }),
      setFavoritesOnly: (favoritesOnly) => set({ favoritesOnly }),
    }),
    { name: "schematadb.treeFilter", partialize: (s) => ({ activeOnly: s.activeOnly, engines: s.engines, favoritesOnly: s.favoritesOnly }) },
  ),
);
