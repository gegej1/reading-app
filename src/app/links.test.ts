import { describe, expect, it } from "vitest";
import { buildKbModel } from "../domain";
import { fixtureKb } from "../domain/__fixtures__";
import {
  docRoute,
  groupLabel,
  makeLinkResolver,
  navSectionForGroup,
  pageRoute,
  safeDecode,
} from "./links";

const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));

describe("routes", () => {
  it("encodes page names and sends 底色 to its own screen", () => {
    expect(pageRoute("小实验")).toBe(`/page/${encodeURIComponent("小实验")}`);
    expect(docRoute({ name: "底色", group: "dise" })).toBe("/dise");
    expect(docRoute({ name: "P-001-速度与退路", group: "precedent" })).toBe(
      pageRoute("P-001-速度与退路"),
    );
  });

  it("decodes parameters safely", () => {
    expect(safeDecode(encodeURIComponent("小实验"))).toBe("小实验");
    expect(safeDecode("100%")).toBe("100%");
    expect(safeDecode(undefined)).toBe("");
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

  it("resolves wikilinks to hash routes", () => {
    expect(resolve("小实验")).toEqual({
      href: `#${pageRoute("小实验")}`,
      title: "wiki/概念/小实验.md",
    });
    expect(resolve("底色")).toEqual({ href: "#/dise", title: "self/底色.md" });
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
