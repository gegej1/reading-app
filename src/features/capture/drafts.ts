// The unsaved text of the capture box, kept in localStorage on every change,
// so a crash or a reload never loses what was typed.

export const DRAFT_KEY = "dise.capture.draft";

export interface DraftStore {
  load(): string;
  save(text: string): void;
  clear(): void;
}

type DraftStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

/** The page's localStorage, or null where there is none (or it is blocked). */
export function pageStorage(): DraftStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

/**
 * Drafts in `storage` under `key` (the capture's own key by default; answers
 * keep one draft per question). Without storage they only live as long as
 * the page.
 */
export function createDraftStore(storage: DraftStorage | null = pageStorage(), key: string = DRAFT_KEY): DraftStore {
  let memory = "";
  return {
    load() {
      try {
        return storage?.getItem(key) ?? memory;
      } catch {
        return memory;
      }
    },
    save(text) {
      memory = text;
      try {
        if (text === "") storage?.removeItem(key);
        else storage?.setItem(key, text);
      } catch {
        // Full or blocked storage: the text is still in the box.
      }
    },
    clear() {
      memory = "";
      try {
        storage?.removeItem(key);
      } catch {
        // See above.
      }
    },
  };
}

/** The draft key of the answer to one question (`Q-001`). */
export function answerDraftKey(rulingId: string): string {
  return `dise.answer.draft.${rulingId}`;
}

/** The draft key of the reason for rejecting one audit item. */
export function rejectionDraftKey(report: string, itemId: string): string {
  return `dise.audit.draft.${report}.${itemId}`;
}

/** A Storage stand-in for tests and previews. */
export function memoryStorage(initial: Record<string, string> = {}): DraftStorage & { snapshot(): Record<string, string> } {
  const items = new Map(Object.entries(initial));
  return {
    getItem: (key) => items.get(key) ?? null,
    setItem: (key, value) => void items.set(key, String(value)),
    removeItem: (key) => void items.delete(key),
    snapshot: () => Object.fromEntries(items),
  };
}
