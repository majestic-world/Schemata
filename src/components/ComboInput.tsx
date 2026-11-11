import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

type Props = {
  value: string;
  options: string[];
  onChange: (value: string) => void;
  label: string;
  className?: string;
};

/** Rows shown before the list scrolls. */
const VISIBLE_ROWS = 10;

/** Options containing `text`, prefix matches first; every option when `text` is empty. */
function matching(options: string[], text: string): string[] {
  const needle = text.toLowerCase();
  if (!needle) return options;
  const starts = options.filter((o) => o.toLowerCase().startsWith(needle));
  return [...starts, ...options.filter((o) => !starts.includes(o) && o.toLowerCase().includes(needle))];
}

/**
 * Free-text input with a compact suggestion list, replacing the browser's oversized `<datalist>` popup.
 * Focusing shows every option; typing narrows them (prefix matches first). Arrows move, Enter picks,
 * Escape closes.
 */
export function ComboInput({ value, options, onChange, label, className = "cell-input" }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const shown = matching(options, filter ?? "");

  const show = (text: string | null) => {
    setRect(input.current?.getBoundingClientRect() ?? null);
    setFilter(text);
    setActive(text ? 0 : Math.max(0, options.findIndex((o) => o.toLowerCase() === value.toLowerCase())));
    setOpen(true);
  };
  const pick = (option: string) => {
    onChange(option);
    setOpen(false);
  };

  useEffect(() => {
    if (open) list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [open, active]);

  // The list is positioned once from the input; scrolling would leave it behind, so close it instead.
  useEffect(() => {
    if (!open) return;
    const close = (e: Event) => !list.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("scroll", close, true);
    return () => window.removeEventListener("scroll", close, true);
  }, [open]);

  return (
    <>
      <input
        ref={input}
        spellCheck={false}
        className={className}
        value={value}
        aria-label={label}
        role="combobox"
        aria-expanded={open}
        aria-autocomplete="list"
        onFocus={() => show(null)}
        onClick={() => !open && show(null)}
        onBlur={() => setOpen(false)}
        onChange={(e) => {
          onChange(e.target.value);
          show(e.target.value);
        }}
        onKeyDown={(e) => {
          if (!open) {
            if (e.key === "ArrowDown") show(null);
            return;
          }
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            const step = e.key === "ArrowDown" ? 1 : -1;
            setActive((a) => Math.min(shown.length - 1, Math.max(0, a + step)));
          } else if (e.key === "Enter" && shown[active]) {
            e.preventDefault();
            pick(shown[active]);
          } else if (e.key === "Escape") {
            e.stopPropagation();
            setOpen(false);
          }
        }}
      />
      {open && rect && shown.length > 0 &&
        createPortal(
          <div
            ref={list}
            className="combo-list"
            role="listbox"
            style={{ left: rect.left, top: rect.bottom + 2, minWidth: rect.width, maxHeight: VISIBLE_ROWS * 22 + 6 }}
            // Keep focus in the input so typing continues while the list is open.
            onMouseDown={(e) => e.preventDefault()}
          >
            {shown.map((o, i) => (
              <div key={o} role="option" aria-selected={i === active} className={`combo-option${i === active ? " active" : ""}`} onMouseEnter={() => setActive(i)} onClick={() => pick(o)}>
                {o}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
