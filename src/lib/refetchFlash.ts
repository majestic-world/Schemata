import { useEffect, useRef, type RefObject } from "react";

/**
 * Briefly fades `target` in when a query of the same `scope` finishes a refetch, so a refresh is visible even when
 * structural sharing keeps the same rows. Switching scope (another database, page, sort or filter) never flashes.
 */
export function useRefetchFlash(target: RefObject<Element | null>, scope: string, dataUpdatedAt: number, isSuccess: boolean) {
  const last = useRef({ scope, at: dataUpdatedAt });
  useEffect(() => {
    const previous = last.current;
    last.current = { scope, at: dataUpdatedAt };
    if (previous.scope !== scope || previous.at === 0 || previous.at === dataUpdatedAt || !isSuccess) return;

    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const animation = target.current?.animate(
      [{ opacity: reducedMotion ? 0.8 : 0.35 }, { opacity: 1 }],
      { duration: reducedMotion ? 120 : 240, easing: "ease-out" },
    );
    return () => animation?.cancel();
  }, [target, scope, dataUpdatedAt, isSuccess]);
}
