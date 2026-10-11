// What a key press in the capture box means. People type Chinese with an IME
// and dictate, so a key that belongs to a composition never saves or hides:
// while composing `isComposing` is set, and WebKit reports the key that
// commits a composition with keyCode 229 after `compositionend` has fired.

export type CaptureKeyAction = "save" | "hide" | null;

export interface CaptureKey {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
  keyCode?: number;
}

/** keyCode of keys that an input method is handling. */
export const IME_KEY_CODE = 229;

/**
 * ⌘↩ (Ctrl+↩ elsewhere) saves, Esc hides; everything else, plain ↩ included,
 * is typing. `composing` is the component's own record of an open composition.
 */
export function captureKeyAction(event: CaptureKey, composing = false): CaptureKeyAction {
  if (composing || event.isComposing || event.keyCode === IME_KEY_CODE) return null;
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.altKey) return "save";
  if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) return "hide";
  return null;
}

/** True on Apple platforms, where the save key is ⌘ rather than Ctrl. */
export function isApplePlatform(nav: Pick<Navigator, "userAgent"> | undefined = globalThis.navigator): boolean {
  return /Mac|iPhone|iPad|iPod/.test(nav?.userAgent ?? "");
}

/** The key hint shown on the slip. */
export function keyHint(apple: boolean): { save: string; hide: string } {
  return apple ? { save: "⌘↩", hide: "esc" } : { save: "Ctrl+↩", hide: "Esc" };
}
