import { create } from "zustand";
import { persist } from "zustand/middleware";

/** Choices in the table designer that carry over to the next table. */
type DesignerPrefs = {
  /** Append `created_at` and `updated_at` DATETIME columns when creating a table. */
  timestamps: boolean;
  setTimestamps: (timestamps: boolean) => void;
};

export const useDesignerPrefs = create<DesignerPrefs>()(
  persist((set) => ({ timestamps: false, setTimestamps: (timestamps) => set({ timestamps }) }), { name: "schematadb.designerPrefs" }),
);
