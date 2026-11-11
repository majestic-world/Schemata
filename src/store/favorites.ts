import { create } from "zustand";
import { persist } from "zustand/middleware";

/** A table or view starred in the schema tree. Names are unique per database across tables and views. */
export type Favorite = { connectionId: string; database: string; name: string };

type FavoritesState = {
  favorites: Favorite[];
  add: (items: readonly Favorite[]) => void;
  remove: (items: readonly Favorite[]) => void;
  rename: (from: Favorite, to: string) => void;
};

const same = (a: Favorite, b: Favorite) => a.connectionId === b.connectionId && a.database === b.database && a.name === b.name;

export const isFavorite = (favorites: readonly Favorite[], item: Favorite) => favorites.some((f) => same(f, item));

export const useFavorites = create<FavoritesState>()(
  persist(
    (set) => ({
      favorites: [],
      add: (items) => set((s) => ({ favorites: [...s.favorites, ...items.filter((i) => !isFavorite(s.favorites, i))] })),
      remove: (items) => set((s) => ({ favorites: s.favorites.filter((f) => !items.some((i) => same(f, i))) })),
      rename: (from, to) => set((s) => ({ favorites: s.favorites.map((f) => (same(f, from) ? { ...f, name: to } : f)) })),
    }),
    { name: "schematadb.favorites" },
  ),
);
