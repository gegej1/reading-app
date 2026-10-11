// @vitest-environment jsdom
// Interactive tests of the capture slip in a DOM: keys, IME composition,
// paste, drafts and failures.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { CapturePanel, type CapturePanelProps } from "./CapturePanel";
import { createDraftStore, DRAFT_KEY, memoryStorage } from "./drafts";
import { IME_KEY_CODE } from "./keys";

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

let mounted: { root: Root; container: HTMLElement } | null = null;

afterEach(() => {
  if (mounted) {
    act(() => mounted?.root.unmount());
    mounted.container.remove();
    mounted = null;
  }
  vi.useRealTimers();
});

function setup(overrides: Partial<CapturePanelProps> = {}, initialDraft?: string) {
  const storage = memoryStorage(initialDraft === undefined ? {} : { [DRAFT_KEY]: initialDraft });
  const saved: string[] = [];
  const props: CapturePanelProps = {
    save: async (text) => {
      saved.push(text);
    },
    hide: vi.fn(),
    drafts: createDraftStore(storage),
    variant: "window",
    confirmMs: 50,
    apple: true,
    ...overrides,
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<CapturePanel {...props} />));
  mounted = { root, container };
  const textarea = container.querySelector("textarea");
  if (!textarea) throw new Error("no textarea");
  return { props, storage, saved, textarea, container };
}

/** Types the whole value, the way a browser reports an input. */
function type(textarea: HTMLTextAreaElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  act(() => {
    setValue?.call(textarea, value);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function press(target: Element, init: KeyboardEventInit & { keyCode?: number }) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  if (init.keyCode !== undefined) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

/** Lets pending promises (the save) settle inside act. */
async function settle(ms = 0) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

const status = (container: HTMLElement) => container.querySelector(".slip__status")?.textContent ?? "";

describe("CapturePanel", () => {
  it("focuses the text when shown", () => {
    const { textarea } = setup({}, "上次没写完");
    expect(document.activeElement).toBe(textarea);
    expect(textarea.selectionStart).toBe("上次没写完".length);
  });

  it("saves on ⌘↩, confirms, clears the draft and hides", async () => {
    const { textarea, saved, storage, props, container } = setup();
    type(textarea, "第一行\n第二行  ");
    const event = press(textarea, { key: "Enter", metaKey: true });
    expect(event.defaultPrevented).toBe(true);
    await settle();
    expect(saved).toEqual(["第一行\n第二行  "]);
    expect(status(container)).toContain("已记下");
    expect(storage.snapshot()).toEqual({});
    expect(props.hide).not.toHaveBeenCalled();
    await settle(80);
    expect(props.hide).toHaveBeenCalledTimes(1);
    expect(textarea.value).toBe("");
  });

  it("saves on Ctrl+↩ too", async () => {
    const { textarea, saved } = setup({ apple: false });
    type(textarea, "一条");
    press(textarea, { key: "Enter", ctrlKey: true });
    await settle();
    expect(saved).toEqual(["一条"]);
  });

  it("does not save on a plain ↩, which stays a new line", async () => {
    const { textarea, saved } = setup();
    type(textarea, "第一行");
    const event = press(textarea, { key: "Enter" });
    await settle();
    expect(event.defaultPrevented).toBe(false);
    expect(saved).toEqual([]);
  });

  it("does not save on ⌘↩ while an input method is composing", async () => {
    const { textarea, saved, container } = setup();
    type(textarea, "zhong");
    act(() => {
      textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    });
    const composing = press(textarea, { key: "Enter", metaKey: true, isComposing: true });
    expect(composing.defaultPrevented).toBe(false);
    // The component's own record is enough, even without isComposing.
    press(textarea, { key: "Enter", metaKey: true });
    act(() => {
      textarea.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "中" }));
    });
    // WebKit: the key that commits the composition arrives with keyCode 229.
    press(textarea, { key: "Enter", metaKey: true, keyCode: IME_KEY_CODE });
    await settle();
    expect(saved).toEqual([]);
    expect(status(container)).not.toContain("已记下");

    // Once the composition is over, ⌘↩ saves.
    type(textarea, "中");
    press(textarea, { key: "Enter", metaKey: true });
    await settle();
    expect(saved).toEqual(["中"]);
  });

  it("does not take a plain ↩ during composition either", async () => {
    const { textarea, saved, props } = setup();
    type(textarea, "ni");
    const enter = press(textarea, { key: "Enter", isComposing: true, keyCode: IME_KEY_CODE });
    const escape = press(textarea, { key: "Escape", isComposing: true, keyCode: IME_KEY_CODE });
    await settle();
    expect(enter.defaultPrevented).toBe(false);
    expect(escape.defaultPrevented).toBe(false);
    expect(saved).toEqual([]);
    expect(props.hide).not.toHaveBeenCalled();
  });

  it("hides on Esc and keeps the draft", async () => {
    const { textarea, props, storage, saved } = setup();
    type(textarea, "还没想好，先放着");
    press(textarea, { key: "Escape" });
    expect(props.hide).toHaveBeenCalledTimes(1);
    expect(saved).toEqual([]);
    expect(textarea.value).toBe("还没想好，先放着");
    expect(storage.snapshot()).toEqual({ [DRAFT_KEY]: "还没想好，先放着" });
    // Opening again (a new page, as after a crash) brings it back.
    act(() => mounted?.root.unmount());
    mounted = null;
    const again = setup({ drafts: createDraftStore(storage) });
    expect(again.textarea.value).toBe("还没想好，先放着");
  });

  it("keeps the text and shows the error when saving fails", async () => {
    const { textarea, props, storage, container } = setup({
      save: async () => {
        throw new Error("知识库正忙：正在同步（已 3 秒），请稍后再试");
      },
    });
    type(textarea, "重要的一条");
    press(textarea, { key: "Enter", metaKey: true });
    await settle(80);
    expect(textarea.value).toBe("重要的一条");
    expect(storage.snapshot()).toEqual({ [DRAFT_KEY]: "重要的一条" });
    expect(props.hide).not.toHaveBeenCalled();
    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("没有记下。知识库正忙");
    // Typing again clears the error.
    type(textarea, "重要的一条！");
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("does not save empty text", async () => {
    const { textarea, saved, container } = setup();
    type(textarea, "  \n\t ");
    press(textarea, { key: "Enter", metaKey: true });
    await settle();
    expect(saved).toEqual([]);
    expect(status(container)).toContain("还没写字");
  });

  it("pastes plain text only", () => {
    const { textarea, storage } = setup();
    type(textarea, "前后");
    act(() => textarea.setSelectionRange(1, 1));
    const paste = new Event("paste", { bubbles: true, cancelable: true }) as Event & {
      clipboardData: { getData(type: string): string };
    };
    paste.clipboardData = {
      getData: (kind: string) => (kind === "text/plain" ? "（中间）" : kind === "text/html" ? "<b>（中间）</b>" : ""),
    };
    act(() => {
      textarea.dispatchEvent(paste);
    });
    expect(paste.defaultPrevented).toBe(true);
    expect(textarea.value).toBe("前（中间）后");
    expect(storage.snapshot()).toEqual({ [DRAFT_KEY]: "前（中间）后" });

    // Nothing but an image on the clipboard: nothing is pasted.
    const image = new Event("paste", { bubbles: true, cancelable: true }) as Event & {
      clipboardData: { getData(type: string): string };
    };
    image.clipboardData = { getData: () => "" };
    act(() => {
      textarea.dispatchEvent(image);
    });
    expect(image.defaultPrevented).toBe(true);
    expect(textarea.value).toBe("前（中间）后");
  });

  it("shows the keys and reports the text to its owner", () => {
    const seen: string[] = [];
    const { textarea, container } = setup({ onTextChange: (text) => seen.push(text) });
    expect(container.querySelector(".slip__keys")?.textContent).toContain("⌘↩");
    type(textarea, "a");
    expect(seen).toEqual(["a"]);
  });
});
