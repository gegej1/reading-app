import { describe, expect, it } from "vitest";
import { parseDise } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { createMemoryStore } from "../../store/memoryStore";
import { editKbFile } from "../../app/editFile";
import { confirmMessage, diseToggle } from "./toggle";

const PATH = "self/底色.md";

describe("confirmMessage", () => {
  it("names the commit after the new state", () => {
    expect(confirmMessage("D05", true)).toBe("底色：确认 D05");
    expect(confirmMessage("D05", false)).toBe("底色：取消确认 D05");
  });
});

describe("diseToggle through editKbFile", () => {
  it("confirms, commits with 底色：确认, and only changes that heading", async () => {
    const store = createMemoryStore(fixtureKb);
    const { edit, message } = diseToggle("D02");
    const result = await editKbFile(store, PATH, edit, message);

    expect(result.message).toBe("底色：确认 D02");
    expect(store.commits).toEqual([{ path: PATH, message: "底色：确认 D02" }]);
    const saved = await store.read(PATH);
    expect(parseDise(saved).entries.find((e) => e.id === "D02")?.confirmed).toBe(true);
    const changed = saved.split("\n").filter((line, i) => line !== fixtureKb[PATH]?.split("\n")[i]);
    expect(changed).toEqual(["### D02 同一时间只做一件主要的事 ✅"]);
  });

  it("un-confirms a confirmed entry and toggling back restores the file", async () => {
    const store = createMemoryStore(fixtureKb);
    const { edit, message } = diseToggle("D05");
    await editKbFile(store, PATH, edit, message);
    await editKbFile(store, PATH, edit, message);
    expect(store.commits.map((c) => c.message)).toEqual(["底色：取消确认 D05", "底色：确认 D05"]);
    expect(await store.read(PATH)).toBe(fixtureKb[PATH]);
  });

  it("re-reads the file, so edits made elsewhere survive", async () => {
    const store = createMemoryStore(fixtureKb);
    const external = `${fixtureKb[PATH]}\n<!-- 外部追加 -->\n`;
    await store.write(PATH, external, "外部修改");
    const { edit, message } = diseToggle("D02");
    await editKbFile(store, PATH, edit, message);
    expect(await store.read(PATH)).toContain("<!-- 外部追加 -->");
  });

  it("does not write when the entry is missing", async () => {
    const store = createMemoryStore(fixtureKb);
    const { edit, message } = diseToggle("D42");
    await expect(editKbFile(store, PATH, edit, message)).rejects.toThrow(/D42/);
    expect(store.commits).toEqual([]);
  });
});
