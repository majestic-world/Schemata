import { create } from "zustand";
import type { ExportSource } from "../api/types";

export type TransferRequest =
  | { kind: "export"; connectionId: string; database: string; source: ExportSource; label: string }
  | { kind: "import"; connectionId: string; database: string; table: string }
  | { kind: "runSql"; connectionId: string; database: string }
  | { kind: "backup"; connectionId: string; database: string }
  | { kind: "restore"; connectionId: string; database: string | null };

type TransferState = { request: TransferRequest | null; open: (r: TransferRequest | null) => void };

export const useTransfer = create<TransferState>((set) => ({ request: null, open: (request) => set({ request }) }));
export const openTransfer = (r: TransferRequest) => useTransfer.getState().open(r);
