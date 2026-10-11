import { describe, expect, it } from "vitest";
import { parseDise } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { createMemoryStore } from "../../store/memoryStore";
import { editKbFile } from "../../app/editFile";
import { confirmEdit, confirmMessage } from "./confirm";

const PATH = "self/底色.md";
const ORIGINAL = fixtureKb[PATH] ?? "";

const confirmedIn = (text: string, id: string) =>
  parseDise(text).entries.find((e) => e.id === id)?.confirmed;

async function apply(store: ReturnType<typeof createMemoryStore>, id: string, desired: boolean) {
  const { edit, message } = confirmEdit(id, desired);
  return editKbFile(store, PATH, edit, message);
}

describe("confirmMessage", () => {
  it("names the commit after the requested state", () => {
    expect(confirmMessage("D05", true)).toBe("底色：确认 D05");
    expect(confirmMessage("D05", false)).toBe("底色：取消确认 D05");
  });
});

describe("confirmEdit through editKbFile", () => {
  it("confirms, commits with 底色：确认, and only changes that heading", async () => {
    const store = createMemoryStore(fixtureKb);
    const result = await apply(store, "D02", true);

    expect(result).toMatchObject({ changed: true, message: "底色：确认 D02" });
    expect(store.commits).toEqual([{ path: PATH, message: "底色：确认 D02" }]);
    const saved = await store.read(PATH);
    expect(confirmedIn(saved, "D02")).toBe(true);
    const before = ORIGINAL.split("\n");
    const changed = saved.split("\n").filter((line, i) => line !== before[i]);
    expect(changed).toEqual(["### D02 同一时间只做一件主要的事 ✅"]);
  });

  it("un-confirms a confirmed entry, and confirming again restores the file", async () => {
    const store = createMemoryStore(fixtureKb);
    await apply(store, "D05", false);
    await apply(store, "D05", true);
    expect(store.commits.map((c) => c.message)).toEqual(["底色：取消确认 D05", "底色：确认 D05"]);
    expect(await store.read(PATH)).toBe(ORIGINAL);
  });

  it("never removes a ✅ because the screen was stale", async () => {
    // The screen still shows D05 as unconfirmed (it was confirmed outside the
    // app, or the last commit failed after the write), so the user clicks
    // 盖章确认. The file keeps its ✅ and nothing is written or committed.
    const store = createMemoryStore(fixtureKb);
    const result = await apply(store, "D05", true);
    expect(result).toEqual({ changed: false, next: ORIGINAL, message: null });
    expect(store.commits).toEqual([]);
    expect(confirmedIn(await store.read(PATH), "D05")).toBe(true);
  });

  it("never adds a ✅ when asked to un-confirm an unconfirmed entry", async () => {
    const store = createMemoryStore(fixtureKb);
    const result = await apply(store, "D02", false);
    expect(result.changed).toBe(false);
    expect(store.commits).toEqual([]);
    expect(confirmedIn(await store.read(PATH), "D02")).toBe(false);
  });

  it("re-reads the file, so edits made elsewhere survive", async () => {
    const store = createMemoryStore(fixtureKb);
    const external = `${ORIGINAL}\n<!-- 外部追加 -->\n`;
    await store.write(PATH, external, "外部修改");
    await apply(store, "D02", true);
    expect(await store.read(PATH)).toContain("<!-- 外部追加 -->");
  });

  it("does not write when the entry is missing", async () => {
    const store = createMemoryStore(fixtureKb);
    await expect(apply(store, "D42", true)).rejects.toThrow(/D42/);
    expect(store.commits).toEqual([]);
  });
});
