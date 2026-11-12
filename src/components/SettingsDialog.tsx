import { useState } from "react";
import { FolderOpen, Moon, Sun } from "lucide-react";
import appIcon from "../assets/app-icon.svg";
import { api } from "../api/client";
import { useAppVersion } from "../lib/useAppVersion";
import { useAppearance } from "../store/appearance";
import { notifyError } from "../store/toasts";
import { Dialog } from "./Dialog";
import { LicensesDialog } from "./LicensesDialog";

export function SettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { theme, setTheme } = useAppearance();
  const version = useAppVersion();
  const [licensesOpen, setLicensesOpen] = useState(false);

  return (
    <Dialog open={open} onClose={onClose} title="Settings" width={460} footer={<button className="btn" onClick={onClose}>Close</button>}>
      <fieldset className="theme-picker">
        <legend>Appearance</legend>
        <p className="hint">Choose a theme. Changes apply immediately and are saved automatically.</p>
        <label className="theme-option">
          <input type="radio" name="theme" value="light" checked={theme === "light"} onChange={() => setTheme("light")} />
          <Sun size={22} aria-hidden />
          <span><strong>Light</strong><span className="hint">Bright surfaces with teal accents</span></span>
        </label>
        <label className="theme-option">
          <input type="radio" name="theme" value="dark" checked={theme === "dark"} onChange={() => setTheme("dark")} />
          <Moon size={22} aria-hidden />
          <span><strong>Dark</strong><span className="hint">Neutral grays with blue accents</span></span>
        </label>
      </fieldset>
      <section className="settings-about" aria-labelledby="settings-about-title">
        <img src={appIcon} width={32} height={32} alt="" />
        <div className="grow">
          <div className="settings-about-title">
            <h2 id="settings-about-title">About Schemata</h2>
            <span className={version.isError ? "error-text" : "muted"} aria-live="polite">
              {version.isError ? "Version unavailable" : version.data ? `v${version.data}` : "…"}
            </span>
          </div>
          <p className="muted">© 2026 Majestic World · MIT License</p>
        </div>
        <button className="btn sm" onClick={() => setLicensesOpen(true)}>Licenses…</button>
      </section>
      <section className="settings-data" aria-labelledby="settings-data-title">
        <div className="grow">
          <h2 id="settings-data-title">App data</h2>
          <p className="hint">Connections, query history and app settings.</p>
        </div>
        <button className="btn sm" title="Open app data folder" onClick={() => api.openAppDataDirectory().catch((e) => notifyError("Could not open app data folder", e))}>
          <FolderOpen size={14} aria-hidden />Open folder
        </button>
      </section>
      <LicensesDialog open={licensesOpen} onClose={() => setLicensesOpen(false)} />
    </Dialog>
  );
}
