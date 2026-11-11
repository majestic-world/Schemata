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

/** `YYYY-MM-DD[ HH:MM[:SS]]` as a local date; null when the text is not such a value. */
function parse(text: string): { day: Date; time: string } | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/.exec(text.trim());
  if (!m) return null;
  const day = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  if (Number.isNaN(day.getTime())) return null;
  return { day, time: m[4] ? `${m[4]}:${m[5]}:${m[6] ?? "00"}` : "00:00:00" };
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
  const initial = parse(value) ?? { day: new Date(now.getFullYear(), now.getMonth(), now.getDate()), time: `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}` };
  const [selected, setSelected] = useState(initial.day);
  const [time, setTime] = useState(initial.time);
  const [view, setView] = useState({ year: initial.day.getFullYear(), month: initial.day.getMonth() });

  const first = new Date(view.year, view.month, 1);
  const days = Array.from({ length: 42 }, (_, i) => new Date(view.year, view.month, 1 - first.getDay() + i));
  const today = ymd(now);
  const pick = (day = selected) => onPick(kind === "datetime" ? `${ymd(day)} ${time.length === 5 ? `${time}:00` : time}` : ymd(day));
  const shiftMonth = (delta: number) => setView((v) => ({ year: new Date(v.year, v.month + delta, 1).getFullYear(), month: new Date(v.year, v.month + delta, 1).getMonth() }));
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
          const d = new Date(now.getFullYear(), now.getMonth(), now.getDate());
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
