/** Full-color icon set in the spirit of classic desktop database tools: toolbar glyphs at 32 px, object
 * glyphs at 16 px. Gradients use per-icon ids so several instances can share a page. */
import type { Engine } from "../api/types";
import { MARIADB_LOGO, MYSQL_LOGO, SQLITE_PARTS, type Logo } from "./engineLogos";

type P = { size?: number };

const Grad = ({ id, from, to, vertical = true }: { id: string; from: string; to: string; vertical?: boolean }) => (
  <linearGradient id={id} x1="0" y1="0" x2={vertical ? "0" : "1"} y2={vertical ? "1" : "0"}>
    <stop offset="0" stopColor={from} />
    <stop offset="1" stopColor={to} />
  </linearGradient>
);

const PlusBadge = ({ x, y, color = "#2fb35a" }: { x: number; y: number; color?: string }) => (
  <g>
    <circle cx={x} cy={y} r="6" fill={color} stroke="#fff" strokeWidth="1.5" />
    <path d={`M${x - 3} ${y}h6M${x} ${y - 3}v6`} stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
  </g>
);

export function ConnectionIcon({ size = 32 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <defs><Grad id="ic-conn" from="#8d99a8" to="#4e5a68" /></defs>
      <path d="M11 3v6M19 3v6" stroke="#9aa5b1" strokeWidth="2.6" strokeLinecap="round" />
      <path d="M6 9h18v5a9 9 0 0 1-9 9 9 9 0 0 1-9-9z" fill="url(#ic-conn)" />
      <path d="M8.5 11h13" stroke="#fff" strokeOpacity=".35" strokeWidth="1.2" />
      <path d="M15 23v3.5a2.5 2.5 0 0 0 2.5 2.5H20" stroke="#4e5a68" strokeWidth="2.4" fill="none" strokeLinecap="round" />
      <PlusBadge x={24.5} y={24.5} />
    </svg>
  );
}

export function NewQueryIcon({ size = 32 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <defs><Grad id="ic-nq" from="#ffffff" to="#e6edf4" /></defs>
      <path d="M6 3h14l6 6v18a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" fill="url(#ic-nq)" stroke="#9fb0c2" />
      <path d="M20 3v6h6" fill="#d3dde7" stroke="#9fb0c2" strokeLinejoin="round" />
      <rect x="4" y="3" width="16" height="4" rx="1.5" fill="#f08a24" />
      <path d="M8 13h12M8 17h9M8 21h7" stroke="#3d7fd6" strokeWidth="1.8" strokeLinecap="round" />
      <PlusBadge x={24} y={24} />
    </svg>
  );
}

export function TableIcon({ size = 32 }: P) {
  const s = size <= 18;
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <defs><Grad id={`ic-tb-${size}`} from="#4aa3ff" to="#1f6fd6" /></defs>
      <rect x="3" y="5" width="26" height="22" rx="3" fill="#eef5fd" stroke="#2b7be0" strokeWidth={s ? 2.4 : 1.4} />
      <path d="M3 8a3 3 0 0 1 3-3h20a3 3 0 0 1 3 3v4H3z" fill={`url(#ic-tb-${size})`} />
      <path d="M3 19.5h26M12 12v15M21 12v15" stroke="#7fb2ee" strokeWidth={s ? 2 : 1.2} />
    </svg>
  );
}

export function ViewIcon({ size = 32 }: P) {
  const s = size <= 18;
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <defs><Grad id={`ic-vw-${size}`} from="#5dc0f5" to="#1d7cc9" /></defs>
      <path d="M3 15c3-2 6-2.5 13-2.5S26 13 29 15" stroke="#1d5f9c" strokeWidth={s ? 2.6 : 1.8} fill="none" strokeLinecap="round" />
      <circle cx="9.5" cy="19" r="5.8" fill={`url(#ic-vw-${size})`} stroke="#1d5f9c" strokeWidth={s ? 2.2 : 1.4} />
      <circle cx="22.5" cy="19" r="5.8" fill={`url(#ic-vw-${size})`} stroke="#1d5f9c" strokeWidth={s ? 2.2 : 1.4} />
      {!s && <path d="M7 16.5a3.5 3.5 0 0 1 3-1.6M20 16.5a3.5 3.5 0 0 1 3-1.6" stroke="#fff" strokeOpacity=".7" strokeWidth="1.3" strokeLinecap="round" />}
    </svg>
  );
}

function Cylinder({ id, from, to, top }: { id: string; from: string; to: string; top: string }) {
  return (
    <>
      <defs><Grad id={id} from={from} to={to} vertical={false} /></defs>
      <path d="M5 8v16c0 2.5 4.9 4.5 11 4.5s11-2 11-4.5V8z" fill={`url(#${id})`} />
      <ellipse cx="16" cy="8" rx="11" ry="4.5" fill={top} />
      <path d="M5 13.5c0 2.5 4.9 4.5 11 4.5s11-2 11-4.5M5 19c0 2.5 4.9 4.5 11 4.5s11-2 11-4.5" stroke="#fff" strokeOpacity=".45" strokeWidth="1.2" fill="none" />
    </>
  );
}

export function DatabaseIcon({ size = 16 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <Cylinder id={`ic-db-${size}`} from="#1f9d55" to="#55c97f" top="#8fe0ad" />
    </svg>
  );
}

export function BackupIcon({ size = 32 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <Cylinder id="ic-bk" from="#6a7b8e" to="#a3b1c0" top="#cfd8e2" />
      <circle cx="24" cy="24" r="6.5" fill="#1d8fe1" stroke="#fff" strokeWidth="1.5" />
      <path d="M21.2 24.4a2.9 2.9 0 1 0 1-2.4" stroke="#fff" strokeWidth="1.6" fill="none" strokeLinecap="round" />
      <path d="M21 20.6v2.2h2.2" stroke="#fff" strokeWidth="1.6" fill="none" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function Tray({ arrow, color }: { arrow: "in" | "out"; color: string }) {
  return (
    <svg width={32} height={32} viewBox="0 0 32 32" aria-hidden>
      <defs><Grad id={`ic-tray-${arrow}`} from="#f4f7fa" to="#d5dee8" /></defs>
      <path d="M4 18h7l2 3h6l2-3h7v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" fill={`url(#ic-tray-${arrow})`} stroke="#8796a7" />
      {arrow === "in" ? (
        <path d="M16 3v13m-5-5 5 5 5-5" stroke={color} strokeWidth="3" fill="none" strokeLinecap="round" strokeLinejoin="round" />
      ) : (
        <path d="M16 16V3m-5 5 5-5 5 5" stroke={color} strokeWidth="3" fill="none" strokeLinecap="round" strokeLinejoin="round" />
      )}
    </svg>
  );
}

export const ImportIcon = () => <Tray arrow="in" color="#22a35a" />;
export const ExportIcon = () => <Tray arrow="out" color="#e9822b" />;

/** Engines shown as a white mark on a colored tile. SQLite uses its own colored logo instead. */
const TILED: Record<Exclude<Engine, "sqlite">, { from: string; to: string; logo: Logo }> = {
  mySql: { from: "#1a8fb4", to: "#00618a", logo: MYSQL_LOGO },
  mariaDb: { from: "#d0a57c", to: "#a9784f", logo: MARIADB_LOGO },
};

function SqliteLogo({ size }: { size: number }) {
  const g = SQLITE_PARTS.gradient;
  return (
    <svg width={size} height={size} viewBox="4 0 124 124" aria-hidden className="engine-icon" shapeRendering="geometricPrecision">
      <defs>
        <linearGradient id="ic-sqlite" x1={g.x1} x2={g.x2} y1={g.y1} y2={g.y2} gradientTransform={g.transform} gradientUnits="userSpaceOnUse">
          {g.stops.map(([color, offset]) => <stop key={offset} stopColor={color} offset={offset} />)}
        </linearGradient>
      </defs>
      {SQLITE_PARTS.paths.map((p) => <path key={p.d.slice(0, 12)} d={p.d} fill={p.fill === "gradient" ? "url(#ic-sqlite)" : p.fill} />)}
    </svg>
  );
}

/** Connection glyph: a tile in the engine's colors carrying the engine's official mark. Drawn on a 16-unit
 * grid so that at the common 16 px size every edge lands on a whole pixel; the mark fills most of the tile
 * (Navicat-style) so it stays legible at that size. */
export function EngineIcon({ engine, size = 16 }: { engine: Engine; size?: number }) {
  if (engine === "sqlite") return <SqliteLogo size={size} />;
  const e = TILED[engine];
  const id = `ic-eng-${engine}`;
  const box = 14.5;
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden className="engine-icon" shapeRendering="geometricPrecision">
      <defs><Grad id={id} from={e.from} to={e.to} /></defs>
      <rect width="16" height="16" rx="3.5" fill={`url(#${id})`} />
      <path
        d={e.logo.path}
        fill="#fff"
        fillRule="evenodd"
        transform={`translate(${(16 - box) / 2} ${(16 - box) / 2}) scale(${box / e.logo.grid})`}
      />
    </svg>
  );
}
