import type { AppError } from "../api/types";

/** The error a cancelled operation ends with, matching the backend's. */
export const cancelledError = (): AppError => ({ kind: "cancelled", message: "Operation cancelled", sqlState: null });

/**
 * Local cancellation intent of one long operation. The backend only cancels operations whose native call has
 * already registered, so a cancel requested earlier (while preparing, or between native calls) is kept here and
 * checked before each native call starts.
 */
export type OperationGate = {
  readonly id: string;
  readonly cancelled: boolean;
  cancel: () => void;
  /** Throws the cancelled error if the operation was cancelled; call it right before starting each native call. */
  checkpoint: () => void;
};

export function operationGate(id: string): OperationGate {
  let cancelled = false;
  return {
    id,
    get cancelled() {
      return cancelled;
    },
    cancel: () => {
      cancelled = true;
    },
    checkpoint: () => {
      if (cancelled) throw cancelledError();
    },
  };
}
