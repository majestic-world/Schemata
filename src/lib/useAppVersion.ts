import { useQuery } from "@tanstack/react-query";
import { getVersion } from "@tauri-apps/api/app";

/** Version Tauri reports for the app (the workspace version in Cargo.toml); shared by the title bar and Settings. */
export function useAppVersion() {
  return useQuery({ queryKey: ["app-version"], queryFn: getVersion, staleTime: Infinity });
}
