import { describe, expect, it } from "vitest";
import { captureKeyAction, IME_KEY_CODE, isApplePlatform, keyHint, type CaptureKey } from "./keys";

const key = (init: Partial<CaptureKey> & { key: string }): CaptureKey => ({
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...init,
});

describe("captureKeyAction", () => {
  it("saves on ⌘↩ and Ctrl+↩ only", () => {
    expect(captureKeyAction(key({ key: "Enter", metaKey: true }))).toBe("save");
    expect(captureKeyAction(key({ key: "Enter", ctrlKey: true }))).toBe("save");
    expect(captureKeyAction(key({ key: "Enter", metaKey: true, shiftKey: true }))).toBe("save");
    expect(captureKeyAction(key({ key: "Enter" }))).toBeNull();
    expect(captureKeyAction(key({ key: "Enter", shiftKey: true }))).toBeNull();
    expect(captureKeyAction(key({ key: "Enter", altKey: true, metaKey: true }))).toBeNull();
  });

  it("hides on a plain Esc", () => {
    expect(captureKeyAction(key({ key: "Escape" }))).toBe("hide");
    expect(captureKeyAction(key({ key: "Escape", metaKey: true }))).toBeNull();
  });

  it("leaves every key of a composition to the input method", () => {
    expect(captureKeyAction(key({ key: "Enter", metaKey: true, isComposing: true }))).toBeNull();
    expect(captureKeyAction(key({ key: "Escape", isComposing: true }))).toBeNull();
    // WebKit: the Enter that commits a composition comes after compositionend.
    expect(captureKeyAction(key({ key: "Enter", metaKey: true, keyCode: IME_KEY_CODE }))).toBeNull();
    expect(captureKeyAction(key({ key: "Escape", keyCode: IME_KEY_CODE }))).toBeNull();
    // The component's own record counts as well.
    expect(captureKeyAction(key({ key: "Enter", ctrlKey: true }), true)).toBeNull();
  });

  it("names the keys the way the platform does", () => {
    expect(isApplePlatform({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)" })).toBe(true);
    expect(isApplePlatform({ userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" })).toBe(false);
    expect(keyHint(true)).toEqual({ save: "⌘↩", hide: "esc" });
    expect(keyHint(false)).toEqual({ save: "Ctrl+↩", hide: "Esc" });
  });
});
