import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import appIcon from "../assets/app-icon.svg";
import { useAppVersion } from "../lib/useAppVersion";

/** Hover time on the maximize button before the Windows 11 Snap Layouts flyout opens, as the native caption does. */
const SNAP_HOVER_MS = 620;

/** Segoe Fluent Icons / MDL2 caption glyphs. */
const GLYPH = { minimize: "\uE921", maximize: "\uE922", restore: "\uE923", close: "\uE8BB" } as const;

/**
 * Caption drawn by the app on Windows (the native one is removed in `src-tauri/src/lib.rs`), so its height and
 * insets stay the same when the window is maximized.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);
  const version = useAppVersion();
  const snapTimer = useRef<number>(undefined);

  useEffect(() => {
    const win = getCurrentWindow();
    const sync = () => void win.isMaximized().then(setMaximized);
    sync();
    const unlisten = [win.onResized(sync), win.onFocusChanged(({ payload }) => setFocused(payload))];
    return () => unlisten.forEach((p) => void p.then((off) => off()));
  }, []);

  const cancelSnap = () => window.clearTimeout(snapTimer.current);
  const win = getCurrentWindow();

  return (
    <div className={`titlebar${focused ? "" : " inactive"}`} data-tauri-drag-region>
      <img className="titlebar-icon" src={appIcon} alt="" draggable={false} />
      <span className="titlebar-title">
        Schemata{version.data && <span className="titlebar-version"> v{version.data}</span>}
      </span>
      <div className="titlebar-controls">
        <button className="caption-btn" tabIndex={-1} aria-label="Minimize" onClick={() => void win.minimize()}>
          {GLYPH.minimize}
        </button>
        <button
          className="caption-btn"
          tabIndex={-1}
          aria-label={maximized ? "Restore" : "Maximize"}
          onClick={() => {
            cancelSnap();
            void win.toggleMaximize();
          }}
          onMouseEnter={() => {
            snapTimer.current = window.setTimeout(() => void win.setFocus().then(() => invoke("plugin:decorum|show_snap_overlay")), SNAP_HOVER_MS);
          }}
          onMouseLeave={cancelSnap}
        >
          {maximized ? GLYPH.restore : GLYPH.maximize}
        </button>
        <button className="caption-btn close" tabIndex={-1} aria-label="Close" onClick={() => void win.close()}>
          {GLYPH.close}
        </button>
      </div>
    </div>
  );
}
