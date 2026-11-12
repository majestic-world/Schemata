import { useState, type FocusEventHandler, type Ref } from "react";
import { createPortal } from "react-dom";
import { ChevronLeft, ChevronRight } from "lucide-react";

export type DateKind = "date" | "datetime";

/** Which picker a column type gets: dates, or dates with a time of day; null for other types. */
export function dateKind(typeName: string): DateKind | null {
  const t = typeName.toLowerCase();
  if (t.startsWith("datetime") || t.startsWith("timestamp")) return "datetime";
  if (t.startsWith("date")) return "date";
  return null;
}

const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
/** Height reserved below the cell before the picker opens above it instead. */
const PICKER_HEIGHT = 300;

const pad = (n: number, width = 2) => String(n).padStart(width, "0");
const ymd = (d: Date) => `${pad(d.getFullYear(), 4)}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** Local midnight of a calendar day (`month` from 0); unlike `new Date(y, m, d)`, years 0–99 stay as written. */
export function localDate(year: number, month: number, day: number): Date {
  const d = new Date(2000, 0, 1);
  d.setFullYear(year, month, day);
  return d;
}

/**
 * `YYYY-MM-DD[ HH:MM[:SS[.fraction]]]` as a local day, its time and its fractional seconds (`.123456`, or empty);
 * null when the text is not such a value or names no real day (`0000-00-00`, `2024-02-30`).
 */
export function parseDateText(text: string): { day: Date; time: string; fraction: string } | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2})(\.\d+)?)?)?/.exec(text.trim());
  if (!m) return null;
  const [year, month, date] = [Number(m[1]), Number(m[2]) - 1, Number(m[3])];
  const day = localDate(year, month, date);
  if (day.getFullYear() !== year || day.getMonth() !== month || day.getDate() !== date) return null;
  return { day, time: m[4] ? `${m[4]}:${m[5]}:${m[6] ?? "00"}` : "00:00:00", fraction: m[7] ?? "" };
}

/** Text of a picked value: the day, and for a datetime the picked time with the fractional seconds the value had. */
export function formatPicked(kind: DateKind, day: Date, time: string, fraction: string): string {
  if (kind === "date") return ymd(day);
  const full = time.length === 5 ? `${time}:00` : time;
  return `${ymd(day)} ${full}${full.includes(".") ? "" : fraction}`;
}

type Props = {
  kind: DateKind;
  /** Current cell text; an empty or unparsable value starts at now. */
  value: string;
  /** Cell the picker opens under. */
  anchor: DOMRect;
  onPick: (text: string) => void;
  onCancel: () => void;
  /** Focus leaving any part of the picker (e.g. the time field). */
  onBlur?: FocusEventHandler<HTMLDivElement>;
  ref?: Ref<HTMLDivElement>;
};

/** Calendar popup for date and datetime cells, like Navicat's: month navigation, Today, a time field, OK/Cancel. */
export function DatePicker({ kind, value, anchor, onPick, onCancel, onBlur, ref }: Props) {
  const now = new Date();
  const initial = parseDateText(value) ?? { day: localDate(now.getFullYear(), now.getMonth(), now.getDate()), time: `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`, fraction: "" };
  const [selected, setSelected] = useState(initial.day);
  const [time, setTime] = useState(initial.time);
  const [view, setView] = useState({ year: initial.day.getFullYear(), month: initial.day.getMonth() });

  const first = localDate(view.year, view.month, 1);
  const days = Array.from({ length: 42 }, (_, i) => localDate(view.year, view.month, 1 - first.getDay() + i));
  const today = ymd(now);
  const pick = (day = selected) => onPick(formatPicked(kind, day, time, initial.fraction));
  const shiftMonth = (delta: number) => setView((v) => {
    const d = localDate(v.year, v.month + delta, 1);
    return { year: d.getFullYear(), month: d.getMonth() };
  });
  const top = anchor.bottom + PICKER_HEIGHT > window.innerHeight ? Math.max(4, anchor.top - PICKER_HEIGHT - 2) : anchor.bottom + 2;

  return createPortal(
    <div
      ref={ref}
      className="datepicker"
      tabIndex={-1}
      onBlur={onBlur}
      style={{ left: Math.min(anchor.left, window.innerWidth - 240), top }}
      // Keep the cell editor focused, except for the time field, which needs the keyboard.
      onMouseDown={(e) => !(e.target instanceof HTMLInputElement) && e.preventDefault()}
      onKeyDown={(e) => {
        // The grid around the editor must not see keys meant for the picker.
        e.stopPropagation();
        if (e.key === "Escape") onCancel();
        if (e.key === "Enter") pick();
      }}
    >
      <div className="datepicker-head">
        <button className="icon-btn" aria-label="Previous month" onClick={() => shiftMonth(-1)}><ChevronLeft size={15} /></button>
        <span>{first.toLocaleDateString("en-US", { month: "long", year: "numeric" })}</span>
        <button className="icon-btn" aria-label="Next month" onClick={() => shiftMonth(1)}><ChevronRight size={15} /></button>
      </div>
      <div className="datepicker-grid">
        {WEEKDAYS.map((w) => <span key={w} className="datepicker-weekday">{w}</span>)}
        {days.map((d) => {
          const key = ymd(d);
          const cls = [d.getMonth() !== view.month && "outside", key === today && "today", key === ymd(selected) && "on"].filter(Boolean).join(" ");
          return (
            <button key={key} className={`datepicker-day ${cls}`} onClick={() => setSelected(d)} onDoubleClick={() => pick(d)}>
              {d.getDate()}
            </button>
          );
        })}
      </div>
      <button
        className="link-btn datepicker-today"
        onClick={() => {
          const d = localDate(now.getFullYear(), now.getMonth(), now.getDate());
          setSelected(d);
          setView({ year: d.getFullYear(), month: d.getMonth() });
        }}
      >
        Today: {today}
      </button>
      {kind === "datetime" && <input className="input sm datepicker-time" type="time" step={1} value={time} aria-label="Time" onChange={(e) => setTime(e.target.value || "00:00:00")} />}
      <div className="datepicker-actions">
        <button className="btn sm primary" onClick={() => pick()}>OK</button>
        <button className="btn sm" onClick={onCancel}>Cancel</button>
      </div>
    </div>,
    document.body,
  );
}
