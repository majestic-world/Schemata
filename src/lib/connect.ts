import type { QueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ConnectionConfig } from "../api/types";
import { useConnections } from "../store/connections";
import { notifyError } from "../store/toasts";

/** Opens a connection, prompting for a password when the server needs one that is not stored. */
export async function openConnection(config: ConnectionConfig, password: string | null = null): Promise<boolean> {
  const conns = useConnections.getState();
  try {
    const version = await api.openConnection(config.id, password);
    conns.markOpen(config.id, version);
    return true;
  } catch (e) {
    const err = e as { kind?: string };
    if (err.kind === "authentication" && password === null) {
      conns.askPassword(config);
    } else {
      notifyError(`Could not connect to ${config.name}`, e);
    }
    return false;
  }
}

export async function closeConnection(id: string, qc: QueryClient) {
  await api.closeConnection(id);
  useConnections.getState().markClosed(id);
  qc.removeQueries({ queryKey: ["databases", id] });
  qc.removeQueries({ queryKey: ["objects", id] });
}
