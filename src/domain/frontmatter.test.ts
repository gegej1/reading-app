import { describe, expect, it } from "vitest";
import {
  fmList,
  fmString,
  parseFrontmatter,
  serializeFrontmatter,
  splitFrontmatter,
} from "./frontmatter";
import { fixture } from "./__fixtures__";

describe("splitFrontmatter", () => {
  it("splits the YAML block from the body and keeps every byte", () => {
    const md = "---\n类型: 判例\n编号: P-001\n---\n\n# 标题\n正文\n";
    const split = splitFrontmatter(md);
    expect(split.yaml).toBe("类型: 判例\n编号: P-001");
    expect(split.body).toBe("\n# 标题\n正文\n");
    expect(split.block + split.body).toBe(md);
  });

  it("returns the whole text as body when there is no frontmatter", () => {
    const md = "# 总目录\n\n- 一条\n";
    const split = splitFrontmatter(md);
    expect(split.yaml).toBeNull();
    expect(split.block).toBe("");
    expect(split.body).toBe(md);
  });

  it("does not treat an unterminated fence as frontmatter", () => {
    const md = "---\n类型: 底色\n\n正文没有结束分隔线\n";
    expect(splitFrontmatter(md).yaml).toBeNull();
    expect(splitFrontmatter(md).body).toBe(md);
  });

  it("does not treat a thematic break in the middle as frontmatter", () => {
    const md = "正文\n\n---\n\n更多正文\n";
    expect(splitFrontmatter(md).yaml).toBeNull();
  });

  it("handles CRLF line endings and an empty block", () => {
    const crlf = "---\r\n类型: 概念\r\n---\r\n正文\r\n";
    const split = splitFrontmatter(crlf);
    expect(split.yaml).toBe("类型: 概念");
    expect(split.body).toBe("正文\r\n");
    expect(split.block + split.body).toBe(crlf);

    const empty = "---\n---\n正文";
    expect(splitFrontmatter(empty).yaml).toBe("");
    expect(splitFrontmatter(empty).body).toBe("正文");
  });

  it("handles a document that is only frontmatter without a final newline", () => {
    const md = "---\n类型: 概念\n---";
    const split = splitFrontmatter(md);
    expect(split.yaml).toBe("类型: 概念");
    expect(split.body).toBe("");
    expect(split.block + split.body).toBe(md);
  });
});

describe("parseFrontmatter", () => {
  it("parses KB-style YAML with dates kept as strings", () => {
    const parsed = parseFrontmatter(fixture("self/判例/P-002-完整与及时.md"));
    expect(parsed.hasFrontmatter).toBe(true);
    expect(parsed.error).toBeNull();
    expect(parsed.data).toEqual({
      类型: "判例",
      编号: "P-002",
      状态: "生效",
      裁决: "2026-09-08",
      关联: "D04、D02",
    });
    expect(parsed.body.startsWith("\n# P-002 完整与及时")).toBe(true);
  });

  it("parses flow sequences such as tags", () => {
    const parsed = parseFrontmatter(fixture("wiki/概念/写作即思考.md"));
    expect(parsed.data["标签"]).toEqual(["做事", "写作"]);
    expect(parsed.data["证据"]).toBe("C");
  });

  it("reports invalid YAML without throwing and keeps the body", () => {
    const md = "---\n类型: [未闭合\n---\n正文\n";
    const parsed = parseFrontmatter(md);
    expect(parsed.hasFrontmatter).toBe(true);
    expect(parsed.data).toEqual({});
    expect(parsed.error).toMatch(/.+/);
    expect(parsed.body).toBe("正文\n");
  });

  it("treats a non-mapping YAML block as empty data", () => {
    const parsed = parseFrontmatter("---\n- a\n- b\n---\n正文");
    expect(parsed.data).toEqual({});
    expect(parsed.error).toMatch(/.+/);
  });

  it("returns empty data for documents without frontmatter", () => {
    const parsed = parseFrontmatter(fixture("index.md"));
    expect(parsed.hasFrontmatter).toBe(false);
    expect(parsed.data).toEqual({});
    expect(parsed.body).toBe(fixture("index.md"));
  });
});

describe("serializeFrontmatter", () => {
  it("round-trips data and keeps the body byte-for-byte", () => {
    const body = "\n# 标题\n\n> 引用\n\n- [ ] 清单\n";
    const data = { 来源: "mac", 时间: "2026-09-10 21:30", 标签: ["a", "b"] };
    const md = serializeFrontmatter(data, body);
    expect(md.startsWith("---\n")).toBe(true);
    const parsed = parseFrontmatter(md);
    expect(parsed.data).toEqual(data);
    expect(parsed.body).toBe(body);
  });

  it("keeps date-like strings as strings", () => {
    const md = serializeFrontmatter({ 更新: "2026-09-10" }, "正文");
    expect(parseFrontmatter(md).data["更新"]).toBe("2026-09-10");
  });

  it("writes only the body when data is empty", () => {
    expect(serializeFrontmatter({}, "正文\n")).toBe("正文\n");
  });
});

describe("fmString / fmList", () => {
  const data = {
    编号: "P-001",
    数字: 3,
    空: null,
    关联: "D04、D02",
    标签: ["人与组织", "招聘"],
  };

  it("coerces scalars to trimmed strings", () => {
    expect(fmString(data, "编号")).toBe("P-001");
    expect(fmString(data, "数字")).toBe("3");
    expect(fmString(data, "空")).toBeNull();
    expect(fmString(data, "缺失")).toBeNull();
    expect(fmString(data, "标签")).toBe("人与组织、招聘");
  });

  it("splits Chinese and ASCII list separators", () => {
    expect(fmList(data, "关联")).toEqual(["D04", "D02"]);
    expect(fmList({ x: "A, B，C;D" }, "x")).toEqual(["A", "B", "C", "D"]);
    expect(fmList(data, "标签")).toEqual(["人与组织", "招聘"]);
    expect(fmList(data, "缺失")).toEqual([]);
  });
});
