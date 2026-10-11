import { describe, expect, it } from "vitest";
import { createDraftStore, DRAFT_KEY, memoryStorage } from "./drafts";

describe("createDraftStore", () => {
  it("keeps the draft verbatim and forgets it when cleared", () => {
    const storage = memoryStorage();
    const drafts = createDraftStore(storage);
    expect(drafts.load()).toBe("");
    drafts.save("  第一行\n\n第二行  ");
    expect(storage.snapshot()).toEqual({ [DRAFT_KEY]: "  第一行\n\n第二行  " });
    // A new page (after a crash) finds it again.
    expect(createDraftStore(storage).load()).toBe("  第一行\n\n第二行  ");
    drafts.clear();
    expect(storage.snapshot()).toEqual({});
    drafts.save("x");
    drafts.save("");
    expect(storage.snapshot()).toEqual({});
  });

  it("still works when storage is missing or throws", () => {
    const memoryOnly = createDraftStore(null);
    memoryOnly.save("只在这一页");
    expect(memoryOnly.load()).toBe("只在这一页");

    const broken = createDraftStore({
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("quota");
      },
      removeItem: () => {
        throw new Error("blocked");
      },
    });
    expect(() => broken.save("还在框里")).not.toThrow();
    expect(broken.load()).toBe("还在框里");
    expect(() => broken.clear()).not.toThrow();
  });
});
