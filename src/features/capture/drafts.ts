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

/** Drafts in `storage`; without storage they only live as long as the page. */
export function createDraftStore(storage: DraftStorage | null = pageStorage()): DraftStore {
  let memory = "";
  return {
    load() {
      try {
        return storage?.getItem(DRAFT_KEY) ?? memory;
      } catch {
        return memory;
      }
    },
    save(text) {
      memory = text;
      try {
        if (text === "") storage?.removeItem(DRAFT_KEY);
        else storage?.setItem(DRAFT_KEY, text);
      } catch {
        // Full or blocked storage: the text is still in the box.
      }
    },
    clear() {
      memory = "";
      try {
        storage?.removeItem(DRAFT_KEY);
      } catch {
        // See above.
      }
    },
  };
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
