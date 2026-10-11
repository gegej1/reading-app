import { describe, expect, it } from "vitest";
import { createDevStore, DEV_READ_ONLY_MESSAGE } from "./devStore";

function fakeFetch(routes: Record<string, () => Response>) {
  const requested: string[] = [];
  const fetch = async (input: string) => {
    requested.push(input);
    const route = routes[input];
    return route ? route() : new Response("未知接口", { status: 404 });
  };
  return { fetch, requested };
}

describe("createDevStore", () => {
  it("lists and reads through the /__kb middleware", async () => {
    const { fetch, requested } = fakeFetch({
      "/__kb/list": () => Response.json([{ path: "self/底色.md", mtime: 3 }]),
      [`/__kb/read?path=${encodeURIComponent("self/底色.md")}`]: () =>
        new Response("# 底色\n"),
    });
    const store = createDevStore({ base: "/", fetch });
    expect(store.kind).toBe("dev");
    expect(await store.list()).toEqual([{ path: "self/底色.md", mtime: 3 }]);
    expect(await store.read("self/底色.md")).toBe("# 底色\n");
    expect(requested).toEqual([
      "/__kb/list",
      "/__kb/read?path=self%2F%E5%BA%95%E8%89%B2.md",
    ]);
  });

  it("respects a non-root base URL", async () => {
    const { fetch, requested } = fakeFetch({
      "/app/__kb/list": () => Response.json([]),
    });
    await createDevStore({ base: "/app", fetch }).list();
    expect(requested).toEqual(["/app/__kb/list"]);
  });

  it("surfaces server errors with their message", async () => {
    const { fetch } = fakeFetch({
      [`/__kb/read?path=${encodeURIComponent("../x.md")}`]: () =>
        new Response("路径不安全", { status: 400 }),
    });
    const store = createDevStore({ base: "/", fetch });
    await expect(store.read("../x.md")).rejects.toThrow(/400.*路径不安全/);
  });

  it("refuses to write", async () => {
    const store = createDevStore({ base: "/", fetch: fakeFetch({}).fetch });
    await expect(store.write("self/底色.md", "x", "m")).rejects.toThrow(DEV_READ_ONLY_MESSAGE);
    await expect(store.create("inbox/a.md", "x", "m")).rejects.toThrow(DEV_READ_ONLY_MESSAGE);
  });
});
