import { describe, expect, it } from "vitest";
import { buildKbModel } from "../domain";
import { fixtureKb } from "../domain/__fixtures__";
import {
  docRoute,
  findPage,
  groupLabel,
  makeLinkResolver,
  navSectionForGroup,
  pageRoute,
  pathFromPageRoute,
  safeDecode,
  sameNamePaths,
} from "./links";

const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));

/** Two files share the name README: the KB asks for unique names, but it happens. */
const twins = buildKbModel([
  { path: "wiki/书/README.md", content: "# 书的说明\n" },
  { path: "wiki/概念/README.md", content: "# 概念的说明\n" },
  {
    path: "wiki/概念/甲.md",
    content: "# 甲\n\n见 [[README]]、[[wiki/概念/README]] 和 [[概念/README|概念说明]]。\n",
  },
]);

const enc = encodeURIComponent;

describe("routes", () => {
  it("puts the encoded repo path in page routes and sends 底色 to its own screen", () => {
    expect(pageRoute("wiki/概念/小实验.md")).toBe(`/page/wiki/${enc("概念")}/${enc("小实验.md")}`);
    // `#`, `?` and `%` in file names must not end the route or break decoding.
    expect(pageRoute("raw/对话/第 1 次 #2?100%.md")).toBe(
      `/page/raw/${enc("对话")}/${enc("第 1 次 #2?100%.md")}`,
    );
    expect(docRoute({ path: "self/底色.md", group: "dise" })).toBe("/dise");
    expect(docRoute({ path: "self/判例/P-001-速度与退路.md", group: "precedent" })).toBe(
      pageRoute("self/判例/P-001-速度与退路.md"),
    );
  });

  it("reads the repo path back from a page URL", () => {
    for (const path of ["wiki/概念/小实验.md", "raw/对话/第 1 次 #2?100%.md", "index.md"]) {
      expect(pathFromPageRoute(pageRoute(path))).toBe(path);
    }
    expect(pathFromPageRoute("/page/bad%E4.md")).toBe("bad%E4.md");
    expect(pathFromPageRoute("/rulings")).toBeNull();
  });

  it("decodes parameters safely", () => {
    expect(safeDecode(encodeURIComponent("小实验"))).toBe("小实验");
    expect(safeDecode("100%")).toBe("100%");
    expect(safeDecode(undefined)).toBe("");
  });
});

describe("findPage", () => {
  it("finds pages by repo path, also when names collide", () => {
    expect(findPage(twins, "wiki/概念/README.md")?.title).toBe("概念的说明");
    expect(findPage(twins, "wiki/书/README.md")?.title).toBe("书的说明");
  });

  it("still opens old name-based page URLs", () => {
    expect(findPage(model, "小实验")?.path).toBe("wiki/概念/小实验.md");
  });

  it("returns null for unknown pages", () => {
    expect(findPage(model, "wiki/没有这一页.md")).toBeNull();
    expect(findPage(model, "")).toBeNull();
  });

  it("lists the other files with the same name", () => {
    const book = twins.byPath.get("wiki/书/README.md");
    expect(book && sameNamePaths(twins, book)).toEqual(["wiki/概念/README.md"]);
    const unique = model.byPath.get("wiki/概念/小实验.md");
    expect(unique && sameNamePaths(model, unique)).toEqual([]);
  });
});

describe("navSectionForGroup", () => {
  it("maps every group to a navigation section", () => {
    expect(navSectionForGroup("dise")).toBe("dise");
    expect(navSectionForGroup("ruling")).toBe("rulings");
    expect(navSectionForGroup("report")).toBe("rulings");
    expect(navSectionForGroup("raw")).toBe("raw");
    expect(navSectionForGroup("wiki")).toBe("knowledge");
    expect(navSectionForGroup("precedent")).toBe("knowledge");
    expect(navSectionForGroup("other")).toBe("knowledge");
  });
});

describe("makeLinkResolver", () => {
  const resolve = makeLinkResolver(model);

  it("resolves wikilinks to path routes", () => {
    expect(resolve("小实验")).toEqual({
      href: `#${pageRoute("wiki/概念/小实验.md")}`,
      title: "wiki/概念/小实验.md",
    });
    expect(resolve("底色")).toEqual({ href: "#/dise", title: "self/底色.md" });
  });

  it("opens the file a path-qualified wikilink names, even when names collide", () => {
    const resolveTwins = makeLinkResolver(twins);
    expect(resolveTwins("wiki/概念/README")?.href).toBe(`#${pageRoute("wiki/概念/README.md")}`);
    expect(resolveTwins("概念/README")?.href).toBe(`#${pageRoute("wiki/概念/README.md")}`);
    expect(resolveTwins("wiki/书/README.md")?.href).toBe(`#${pageRoute("wiki/书/README.md")}`);
  });

  it("resolves an ambiguous bare name deterministically and says so", () => {
    const resolveTwins = makeLinkResolver(twins);
    // First path in code-point order, every time.
    expect(resolveTwins("README")).toEqual({
      href: `#${pageRoute("wiki/书/README.md")}`,
      title: "wiki/书/README.md（同名文件共 2 个）",
    });
  });

  it("returns null for broken links", () => {
    expect(resolve("没有这一页")).toBeNull();
  });
});

describe("groupLabel", () => {
  it("names groups in Chinese", () => {
    expect(groupLabel({ group: "wiki", subgroup: "概念" })).toBe("概念");
    expect(groupLabel({ group: "raw", subgroup: "inbox" })).toBe("收件箱");
    expect(groupLabel({ group: "raw", subgroup: "books" })).toBe("书");
    expect(groupLabel({ group: "system", subgroup: null })).toBe("体系");
    expect(groupLabel({ group: "precedent", subgroup: null })).toBe("判例");
  });
});
