import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ConnectionConfig } from "../api/types";

export function useConnection(id: string | undefined): ConnectionConfig | undefined {
  const connections = useQuery({ queryKey: ["connections"], queryFn: api.listConnections });
  return connections.data?.find((c) => c.id === id);
}
