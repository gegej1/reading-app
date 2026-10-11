import { createContext, useContext } from "react";
import type { KbModel } from "../domain";
import type { EditResult } from "./editFile";
import type { KbStore } from "../store/types";
import type { LinkResolver } from "../ui/Markdown";

export interface KbContextValue {
  store: KbStore;
  model: KbModel;
  /** False for the read-only dev preview. */
  writable: boolean;
  /** A reload is running (the current model stays visible meanwhile). */
  reloading: boolean;
  /**
   * An edit is queued or being written (or the reload after it is running).
   * Write controls stay disabled meanwhile.
   */
  editing: boolean;
  reload(): Promise<void>;
  /**
   * Re-reads `path` from the store, applies `edit`, and writes the result with
   * the commit message returned by `message` (nothing is written when the edit
   * changes nothing). All edits share one queue, so they never interleave.
   * The model is reloaded after every attempt, success or failure.
   */
  editFile(
    path: string,
    edit: (current: string) => string,
    message: (next: string) => string,
  ): Promise<EditResult>;
  /** Wikilink resolver for the current model. */
  resolveLink: LinkResolver;
  /** One-line status message shown briefly at the bottom of the window. */
  notice: string | null;
  notify(text: string): void;
  /** Human-readable source of the data (store label, KB folder). */
  sourceLabel: string;
  sourceDetail: string | null;
}

export const KbContext = createContext<KbContextValue | null>(null);

/** The loaded knowledge base. Only usable below a ready KbProvider. */
export function useKb(): KbContextValue {
  const value = useContext(KbContext);
  if (!value) throw new Error("useKb must be used inside <KbProvider>");
  return value;
}
