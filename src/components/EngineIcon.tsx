import { Database } from "lucide-react";
import type { Engine } from "../api/types";

const ENGINE_COLOR: Record<Engine, string> = { mySql: "#00758f", mariaDb: "#b2603d", sqlite: "#2f7fc1" };

export function EngineIcon({ engine, size = 16 }: { engine: Engine; size?: number }) {
  return (
    <span className="engine-icon" style={{ background: ENGINE_COLOR[engine] }}>
      <Database size={size - 4} color="#fff" strokeWidth={2.2} />
    </span>
  );
}
