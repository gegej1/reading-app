import { describe, expect, it } from "vitest";
import { createSlugger, findAnchorTarget, headingSlug } from "./anchors";

describe("headingSlug", () => {
  it("keeps CJK text, lower-cases Latin and joins words with hyphens", () => {
    expect(headingSlug("小节")).toBe("小节");
    expect(headingSlug("Next Step!")).toBe("next-step");
    expect(headingSlug("和底色的关系")).toBe("和底色的关系");
    expect(headingSlug("  P-001 速度与退路  ")).toBe("p-001-速度与退路");
  });

  it("drops punctuation and symbols such as 、 and ✅", () => {
    expect(headingSlug("一、怎么做选择")).toBe("一怎么做选择");
    expect(headingSlug("D01 先留退路 ✅")).toBe("d01-先留退路");
    expect(headingSlug("（草稿，待你确认）")).toBe("草稿待你确认");
  });

  it("never returns an empty id", () => {
    expect(headingSlug("✅")).toBe("section");
    expect(headingSlug("")).toBe("section");
  });
});

describe("createSlugger", () => {
  it("numbers repeated headings in document order", () => {
    const slug = createSlugger();
    expect(["小节", "要点", "小节", "小节", "要点"].map(slug)).toEqual([
      "小节",
      "要点",
      "小节-1",
      "小节-2",
      "要点-1",
    ]);
  });

  it("does not collide with a heading that already ends in a number", () => {
    const slug = createSlugger();
    expect(["小节-1", "小节", "小节"].map(slug)).toEqual(["小节-1", "小节", "小节-2"]);
  });
});

describe("findAnchorTarget", () => {
  const headings = [
    { id: "一怎么做选择", textContent: "一、怎么做选择" },
    { id: "小节", textContent: "小节" },
    { id: "小节-1", textContent: "小节" },
    { id: "next-step", textContent: "Next Step!" },
  ];

  it("finds a heading by id, by slug or by its text", () => {
    expect(findAnchorTarget(headings, "小节")).toBe(headings[1]);
    expect(findAnchorTarget(headings, "小节-1")).toBe(headings[2]);
    expect(findAnchorTarget(headings, "一、怎么做选择")).toBe(headings[0]);
    expect(findAnchorTarget(headings, "Next Step")).toBe(headings[3]);
  });

  it("accepts percent-encoded fragments", () => {
    expect(findAnchorTarget(headings, encodeURIComponent("小节"))).toBe(headings[1]);
  });

  it("returns null when nothing matches", () => {
    expect(findAnchorTarget(headings, "没有")).toBeNull();
    expect(findAnchorTarget(headings, "")).toBeNull();
  });
});
