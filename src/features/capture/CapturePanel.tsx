import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
} from "react";
import { isBlankCapture } from "../../domain/capture";
import { cx } from "../../ui/components";
import type { DraftStore } from "./drafts";
import { captureKeyAction, isApplePlatform, keyHint } from "./keys";

type Phase = { kind: "idle" } | { kind: "saving" } | { kind: "saved" } | { kind: "error"; message: string };

/** The words on a slip. The capture's by default; answers bring their own. */
export interface SlipCopy {
  title: string;
  placeholder: string;
  /** The status line before anything happened. */
  idle: string;
  saving: string;
  saved: string;
  /** Before the reason of a failure. */
  failed: string;
  /** The save button. */
  save: string;
  /** The text box's name for screen readers. */
  label: string;
}

export const CAPTURE_COPY: SlipCopy = {
  title: "记一笔",
  placeholder: "想到什么，先记下来……",
  idle: "放进收件箱，下次入库时整理",
  saving: "正在记下…",
  saved: "已记下",
  failed: "没有记下。",
  save: "记下",
  label: "随手记一笔",
};

export interface CapturePanelProps {
  /** Saves the text in the knowledge base; rejects with a readable message. */
  save(text: string): Promise<void>;
  /**
   * Puts the slip away (Esc, 收起). The draft stays, unless it was just
   * saved. Without it the slip stays where it is (inline in a page).
   */
  hide?(): void;
  /** Where the unsaved text is kept. */
  drafts: DraftStore;
  /** Changes every time the slip is shown, so the text gets the focus again. */
  shownAt?: number;
  /** `window`: the floating capture window; `dialog`: inside the page; `inline`: part of a page. */
  variant: "window" | "dialog" | "inline";
  /** How long 已记下 stays before the slip goes away (or, inline, empties). */
  confirmMs?: number;
  /** Told about every change of the text (the window hides on blur only when it is empty). */
  onTextChange?(text: string): void;
  /** Use ⌘ in the key hint (default: detected from the platform). */
  apple?: boolean;
  /** Other words for the slip (answers, reasons). */
  copy?: Partial<SlipCopy>;
  /** Why saving is not possible right now; the text stays editable and its draft is kept. */
  blocked?: string | null;
  /** Called once a save went through. */
  onSaved?(): void;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * The capture slip (记一笔): one text box. ⌘↩ (Ctrl+↩) saves, ↩ is a new
 * line, Esc puts the slip away and keeps the draft. Keys that belong to an
 * input-method composition are left alone, pasting inserts plain text only,
 * and the draft is stored on every change.
 */
export function CapturePanel({
  save,
  hide,
  drafts,
  shownAt,
  variant,
  confirmMs = 900,
  onTextChange,
  apple = isApplePlatform(),
  copy: wording,
  blocked = null,
  onSaved,
}: CapturePanelProps) {
  const copy = { ...CAPTURE_COPY, ...wording };
  const [text, setText] = useState(() => drafts.load());
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const area = useRef<HTMLTextAreaElement>(null);
  const composing = useRef(false);
  const caret = useRef<number | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const hint = keyHint(apple);
  const busy = phase.kind === "saving" || phase.kind === "saved";
  const inline = variant === "inline";

  useEffect(() => () => clearTimeout(timer.current), []);

  // Focus the text, caret at the end, every time the slip is shown. A slip
  // that is part of a page only takes the focus when asked to (shownAt).
  useEffect(() => {
    const box = area.current;
    if (!box || (inline && shownAt === undefined)) return;
    box.focus();
    box.setSelectionRange(box.value.length, box.value.length);
  }, [shownAt, inline]);

  // After a paste that had to be applied by hand, put the caret behind it.
  useLayoutEffect(() => {
    if (caret.current === null || !area.current) return;
    area.current.setSelectionRange(caret.current, caret.current);
    caret.current = null;
  }, [text]);

  const change = useCallback(
    (next: string) => {
      setText(next);
      drafts.save(next);
      onTextChange?.(next);
      setPhase((current) => (current.kind === "error" ? { kind: "idle" } : current));
    },
    [drafts, onTextChange],
  );

  const submit = async () => {
    if (busy || blocked !== null) return;
    if (isBlankCapture(text)) {
      setPhase({ kind: "error", message: "还没写字" });
      area.current?.focus();
      return;
    }
    setPhase({ kind: "saving" });
    try {
      await save(text);
    } catch (error) {
      // The text stays in the box and in the draft.
      setPhase({ kind: "error", message: errorText(error) });
      return;
    }
    drafts.clear();
    setPhase({ kind: "saved" });
    onSaved?.();
    timer.current = setTimeout(() => {
      setText("");
      onTextChange?.("");
      setPhase({ kind: "idle" });
      hide?.();
    }, confirmMs);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const native = event.nativeEvent;
    const action = captureKeyAction(
      {
        key: event.key,
        metaKey: event.metaKey,
        ctrlKey: event.ctrlKey,
        altKey: event.altKey,
        shiftKey: event.shiftKey,
        isComposing: native.isComposing,
        keyCode: native.keyCode,
      },
      composing.current,
    );
    if (action === "save") {
      event.preventDefault();
      void submit();
    } else if (action === "hide" && hide) {
      event.preventDefault();
      hide();
    }
  };

  // Plain text only: formatting, images and files on the clipboard are ignored.
  const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const plain = event.clipboardData.getData("text/plain");
    event.preventDefault();
    if (!plain || busy) return;
    const box = event.currentTarget;
    // insertText keeps the undo history and fires the usual input event.
    if (typeof document.execCommand === "function" && document.execCommand("insertText", false, plain)) return;
    const start = box.selectionStart;
    const end = box.selectionEnd;
    caret.current = start + plain.length;
    change(box.value.slice(0, start) + plain + box.value.slice(end));
  };

  const status =
    phase.kind === "saving"
      ? copy.saving
      : phase.kind === "saved"
        ? copy.saved
        : phase.kind === "error"
          ? `${copy.failed}${phase.message}`
          : (blocked ?? copy.idle);

  return (
    <div
      className={cx("slip", `slip--${variant}`, phase.kind === "saved" && "is-saved", blocked !== null && "is-blocked")}
      onKeyDown={onKeyDown}
    >
      <header className="slip__head">
        <span className="slip__title">{copy.title}</span>
        <span className="slip__keys" aria-hidden="true">
          <kbd>{hint.save}</kbd> {copy.save}
          {hide ? (
            <>
              {" "}
              · <kbd>{hint.hide}</kbd> 收起
            </>
          ) : null}
        </span>
      </header>
      <textarea
        ref={area}
        className="slip__text"
        value={text}
        onChange={(event) => change(event.target.value)}
        onPaste={onPaste}
        onCompositionStart={() => (composing.current = true)}
        onCompositionEnd={() => (composing.current = false)}
        readOnly={busy}
        placeholder={copy.placeholder}
        aria-label={copy.label}
        spellCheck={false}
        autoComplete="off"
      />
      <footer className="slip__foot">
        <p
          className={cx("slip__status", `is-${phase.kind}`, blocked !== null && phase.kind === "idle" && "is-blocked")}
          role={phase.kind === "error" ? "alert" : "status"}
          aria-live="polite"
        >
          {phase.kind === "saved" ? <span className="slip__check" aria-hidden="true" /> : null}
          {status}
        </p>
        {hide ? (
          <button type="button" className="btn btn--quiet btn--small" onClick={hide}>
            收起
          </button>
        ) : null}
        <button
          type="button"
          className="btn btn--seal btn--small"
          onClick={() => void submit()}
          disabled={busy || blocked !== null}
          title={blocked ?? undefined}
        >
          {copy.save}
        </button>
      </footer>
    </div>
  );
}
