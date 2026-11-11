export const isMac = /mac/i.test(navigator.platform) || /mac os/i.test(navigator.userAgent);
export const isWindows = /win/i.test(navigator.platform);
/** Label of the primary modifier key: ⌘ on macOS, Ctrl elsewhere. */
export const MOD = isMac ? "⌘" : "Ctrl";
export const isMod = (e: { ctrlKey: boolean; metaKey: boolean }) => (isMac ? e.metaKey : e.ctrlKey);
