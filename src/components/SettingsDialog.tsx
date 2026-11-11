import { useState } from "react";
import { Moon, Sun } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { getVersion } from "@tauri-apps/api/app";
import appIcon from "../assets/app-icon.svg";
import { useAppearance } from "../store/appearance";
import { Dialog } from "./Dialog";
import { LicensesDialog } from "./LicensesDialog";

export function SettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { theme, setTheme } = useAppearance();
  const version = useQuery({ queryKey: ["app-version"], queryFn: getVersion, enabled: open, staleTime: Infinity });
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
          <h2 id="settings-about-title">About Schemata</h2>
          <p className={version.isError ? "error-text" : "muted"} aria-live="polite">
            {version.isError ? "Could not load app version." : version.data ? `Version ${version.data}` : "Loading version…"}
          </p>
          <p className="muted">© 2026 Majestic World · MIT License</p>
        </div>
        <button className="btn sm" onClick={() => setLicensesOpen(true)}>Licenses…</button>
      </section>
      <LicensesDialog open={licensesOpen} onClose={() => setLicensesOpen(false)} />
    </Dialog>
  );
}
