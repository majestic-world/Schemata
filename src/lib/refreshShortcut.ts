import { useEffect, useRef, type RefObject } from "react";

/**
 * F5 / Ctrl+R refresh the data of the view in front of the user instead of reloading the webview.
 * Views register a handler while mounted; the most recently mounted one wins (only the active tab's
 * view is mounted).
 */
const handlers: RefObject<() => void>[] = [];

export function isRefreshShortcut(e: KeyboardEvent): boolean {
  if (e.key === "F5") return true;
  return (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "r";
}

export function runRefreshHandler(): void {
  handlers.at(-1)?.current();
}

export function useRefreshShortcut(handler: () => void): void {
  const ref = useRef(handler);
  useEffect(() => {
    ref.current = handler;
  });
  useEffect(() => {
    handlers.push(ref);
    return () => {
      handlers.splice(handlers.indexOf(ref), 1);
    };
  }, []);
}
