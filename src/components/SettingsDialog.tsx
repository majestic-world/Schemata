import { Moon, Sun } from "lucide-react";
import { useAppearance } from "../store/appearance";
import { Dialog } from "./Dialog";

export function SettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { theme, setTheme } = useAppearance();

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
    </Dialog>
  );
}
