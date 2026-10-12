// Saving answers (回答) and rejections of audit items (不认可): one new file
// in inbox/裁决/ each, created through the store like a capture, so nothing
// is ever overwritten. Desktop answers say 来源: mac, everything else web.

import { useCallback } from "react";
import { buildAnswer, saveCapture, type AnswerTarget, type CaptureSource, type SavedCapture } from "../domain";
import { useKb } from "./KbContext";
import { desktop, type Desktop } from "./desktop";

/** Where an answer is made: the Mac app on its own knowledge base, or the web. */
export function answerSource(bridge: Pick<Desktop, "active">, storeKind: string): CaptureSource {
  return bridge.active && storeKind === "tauri" ? "mac" : "web";
}

/** Saves one answer and reloads the knowledge base. Rejects with a readable message. */
export function useSaveAnswer(bridge: Desktop = desktop) {
  const { store, reload } = useKb();
  const source = answerSource(bridge, store.kind);
  return useCallback(
    async (target: AnswerTarget, text: string): Promise<SavedCapture> => {
      const saved = await saveCapture(store, buildAnswer({ target, text, source, now: new Date() }));
      bridge.log("info", `answer saved: ${saved.path}`);
      await reload();
      return saved;
    },
    [store, source, reload, bridge],
  );
}

/** `2026-10-12T14:03:22+08:00` or `2026-09-12 22:10` → `10 月 12 日 14:03`; anything else as it is. */
export function formatAnswerTime(time: string | null): string {
  if (!time) return "";
  const match = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2}))?/.exec(time.trim());
  if (!match) return time;
  const day = `${Number(match[2])} 月 ${Number(match[3])} 日`;
  return match[4] ? `${day} ${match[4]}:${match[5]}` : day;
}
