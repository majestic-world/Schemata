import { useRef, useState } from "react";

type Props = {
  /** Current width of the panel being resized, in pixels. */
  value: number;
  onChange: (next: number) => void;
  min: number;
  max: number;
  /** "left": dragging right grows the panel. "right": dragging left grows it. */
  side: "left" | "right";
};

export function Splitter({ value, onChange, min, max, side }: Props) {
  const [dragging, setDragging] = useState(false);
  const start = useRef({ x: 0, value: 0 });

  return (
    <div
      className={`splitter${dragging ? " dragging" : ""}`}
      role="separator"
      aria-orientation="vertical"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { x: e.clientX, value };
        setDragging(true);
      }}
      onPointerMove={(e) => {
        if (!dragging) return;
        const delta = e.clientX - start.current.x;
        const next = start.current.value + (side === "left" ? delta : -delta);
        onChange(Math.min(max, Math.max(min, next)));
      }}
      onPointerUp={() => setDragging(false)}
    />
  );
}
