import { describe, expect, it } from "vitest";
import { parseDise, setConfirmed } from "../domain";
import { fixtureKb } from "../domain/__fixtures__";
import { createMemoryStore, type MemoryStore } from "../store/memoryStore";
import { createKbEditor, createSerialQueue, editKbFile } from "./editFile";

const PATH = "self/底色.md";
const ORIGINAL = fixtureKb[PATH] ?? "";

/** A memory store whose reads and writes take a while, like disk access plus git. */
function withLatency(store: MemoryStore, ms: number, log?: string[]): MemoryStore {
  const pause = () => new Promise<void>((resolve) => setTimeout(resolve, ms));
  return {
    ...store,
    async read(path) {
      log?.push("read");
      const text = await store.read(path);
      await pause();
      return text;
    },
    async write(path, content, message) {
      log?.push("write");
      await pause();
      await store.write(path, content, message);
    },
  };
}

const confirmed = (text: string, id: string) =>
  parseDise(text).entries.find((entry) => entry.id === id)?.confirmed;

const confirm = (id: string) => [
  PATH,
  (text: string) => setConfirmed(text, id, true),
  () => `底色：确认 ${id}`,
] as const;

describe("editKbFile", () => {
  it("writes and commits a changed file", async () => {
    const store = createMemoryStore(fixtureKb);
    const result = await editKbFile(store, ...confirm("D02"));
    expect(result.changed).toBe(true);
    expect(result.message).toBe("底色：确认 D02");
    expect(store.commits).toEqual([{ path: PATH, message: "底色：确认 D02" }]);
  });

  it("does not write or commit when the edit changes nothing", async () => {
    const store = createMemoryStore(fixtureKb);
    // D05 is already confirmed in the file.
    const result = await editKbFile(store, ...confirm("D05"));
    expect(result).toEqual({ changed: false, next: ORIGINAL, message: null });
    expect(store.commits).toEqual([]);
  });

  it("is not safe on its own: overlapping read-modify-writes lose an update", async () => {
    // Documents the race the editor queue exists for (and proves the latency
    // store really interleaves the two cycles).
    const store = withLatency(createMemoryStore(fixtureKb), 5);
    await Promise.all([editKbFile(store, ...confirm("D02")), editKbFile(store, ...confirm("D03"))]);
    const saved = await store.read(PATH);
    expect([confirmed(saved, "D02"), confirmed(saved, "D03")]).toContain(false);
  });
});

describe("createSerialQueue", () => {
  it("runs tasks one at a time in call order, even after a failure", async () => {
    const enqueue = createSerialQueue();
    const events: string[] = [];
    const task = (name: string, ms: number, fail = false) => async () => {
      events.push(`${name} start`);
      await new Promise((resolve) => setTimeout(resolve, ms));
      events.push(`${name} end`);
      if (fail) throw new Error(`${name} failed`);
      return name;
    };
    const a = enqueue(task("a", 6, true));
    const b = enqueue(task("b", 1));
    const c = enqueue(task("c", 1));
    await expect(a).rejects.toThrow("a failed");
    await expect(b).resolves.toBe("b");
    await expect(c).resolves.toBe("c");
    expect(events).toEqual(["a start", "a end", "b start", "b end", "c start", "c end"]);
  });
});

describe("createKbEditor", () => {
  it("serializes concurrent edits so both land", async () => {
    const store = withLatency(createMemoryStore(fixtureKb), 5);
    const editFile = createKbEditor(store);
    const results = await Promise.all([editFile(...confirm("D02")), editFile(...confirm("D03"))]);

    expect(results.map((r) => r.changed)).toEqual([true, true]);
    const saved = await store.read(PATH);
    expect(confirmed(saved, "D02")).toBe(true);
    expect(confirmed(saved, "D03")).toBe(true);
    expect(store.commits.map((c) => c.message)).toEqual(["底色：确认 D02", "底色：确认 D03"]);
  });

  it("reloads after every attempt, before the next edit starts", async () => {
    const log: string[] = [];
    const store = withLatency(createMemoryStore(fixtureKb), 2, log);
    const editFile = createKbEditor(store, {
      afterEdit: async () => {
        log.push("reload");
      },
    });
    const broken = editFile(
      PATH,
      () => {
        throw new Error("底色里没有找到条目 D42");
      },
      () => "不会提交",
    );
    const fine = editFile(...confirm("D02"));
    await expect(broken).rejects.toThrow("D42");
    await expect(fine).resolves.toMatchObject({ changed: true });
    expect(log).toEqual(["read", "reload", "read", "write", "reload"]);
  });

  it("reloads and reports the error when the write fails half-way", async () => {
    const store = createMemoryStore(fixtureKb);
    let reloads = 0;
    const editFile = createKbEditor(
      {
        ...store,
        async write() {
          throw new Error("文件已写入，但 git 提交失败：index.lock exists");
        },
      },
      {
        afterEdit: async () => {
          reloads += 1;
        },
      },
    );
    await expect(editFile(...confirm("D02"))).rejects.toThrow("git 提交失败");
    expect(reloads).toBe(1);
  });

  it("keeps the edit's own outcome when the reload fails", async () => {
    const store = createMemoryStore(fixtureKb);
    const editFile = createKbEditor(store, {
      afterEdit: async () => {
        throw new Error("重新载入失败");
      },
    });
    await expect(editFile(...confirm("D02"))).resolves.toMatchObject({ changed: true });
  });

  it("reports how many edits are queued or running", async () => {
    const pending: number[] = [];
    const editFile = createKbEditor(withLatency(createMemoryStore(fixtureKb), 2), {
      onPendingChange: (count) => pending.push(count),
    });
    await Promise.all([editFile(...confirm("D02")), editFile(...confirm("D03"))]);
    expect(pending).toEqual([1, 2, 1, 0]);
  });
});
