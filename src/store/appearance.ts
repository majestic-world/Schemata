import { create } from "zustand";
import { persist } from "zustand/middleware";

type Theme = "light" | "dark";

type AppearanceState = {
  theme: Theme;
  setTheme: (theme: Theme) => void;
};

export const useAppearance = create<AppearanceState>()(
  persist(
    (set) => ({
      theme: "light",
      setTheme: (theme) => set({ theme }),
    }),
    { name: "schematadb.appearance" },
  ),
);
