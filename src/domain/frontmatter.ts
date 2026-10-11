import { Document, isSeq, parseDocument, visit } from "yaml";

/** Parsed YAML frontmatter. Keys are whatever the page uses (mostly Chinese). */
export type FrontmatterData = Record<string, unknown>;

export interface FrontmatterSplit {
  /** YAML text between the fences, or null when the document has no frontmatter. */
  yaml: string | null;
  /** The whole frontmatter block: both fences and the line break after the closing one. */
  block: string;
  /** Everything after the block, byte-for-byte. `block + body` is always the input. */
  body: string;
}

export interface ParsedFrontmatter {
  hasFrontmatter: boolean;
  data: FrontmatterData;
  /** Parse problem (invalid YAML or not a mapping); data is then empty. */
  error: string | null;
  block: string;
  body: string;
}

const OPENING_FENCE = /^\uFEFF?---[ \t]*\r?\n/;
const CLOSING_FENCE = /^---[ \t]*$/;

/** Splits a document into its frontmatter block and body without touching either. */
export function splitFrontmatter(text: string): FrontmatterSplit {
  const opening = OPENING_FENCE.exec(text);
  if (!opening) {
    return { yaml: null, block: "", body: text };
  }

  const yamlStart = opening[0].length;
  let lineStart = yamlStart;
  for (;;) {
    const newline = text.indexOf("\n", lineStart);
    const lineEnd = newline === -1 ? text.length : newline;
    const line = text.slice(lineStart, lineEnd).replace(/\r$/, "");
    if (CLOSING_FENCE.test(line)) {
      const blockEnd = newline === -1 ? text.length : newline + 1;
      return {
        yaml: text.slice(yamlStart, lineStart).replace(/\r?\n$/, ""),
        block: text.slice(0, blockEnd),
        body: text.slice(blockEnd),
      };
    }
    if (newline === -1) {
      return { yaml: null, block: "", body: text };
    }
    lineStart = newline + 1;
  }
}

/** Parses the frontmatter. Never throws: problems are reported in `error`. */
export function parseFrontmatter(text: string): ParsedFrontmatter {
  const split = splitFrontmatter(text);
  if (split.yaml === null) {
    return { hasFrontmatter: false, data: {}, error: null, block: "", body: text };
  }

  const result = (data: FrontmatterData, error: string | null) => ({
    hasFrontmatter: true,
    data,
    error,
    block: split.block,
    body: split.body,
  });

  // YAML 1.2 core schema: dates such as 2026-10-11 stay strings.
  const doc = parseDocument(split.yaml, { uniqueKeys: false });
  if (doc.errors.length > 0) {
    return result({}, doc.errors[0]?.message ?? "YAML 解析失败");
  }
  const value: unknown = doc.toJS();
  if (value === null || value === undefined) {
    return result({}, null);
  }
  if (typeof value !== "object" || Array.isArray(value)) {
    return result({}, "frontmatter 不是键值对");
  }
  return result(value as FrontmatterData, null);
}

/**
 * Builds a document from frontmatter data and a body. Lists of scalars use the
 * KB's flow style (`标签: [甲, 乙]`). The body is appended unchanged.
 */
export function serializeFrontmatter(data: FrontmatterData, body: string): string {
  if (Object.keys(data).length === 0) {
    return body;
  }
  const doc = new Document(data);
  visit(doc, {
    Seq(_key, node) {
      if (isSeq(node) && node.items.every((item) => !isSeq(item))) {
        node.flow = true;
      }
    },
  });
  const yaml = doc.toString({ lineWidth: 0, flowCollectionPadding: false });
  return `---\n${yaml}---\n${body}`;
}

/** Reads a frontmatter value as a trimmed string (lists are joined with 、). */
export function fmString(data: FrontmatterData, key: string): string | null {
  const value = data[key];
  if (value === null || value === undefined) {
    return null;
  }
  if (Array.isArray(value)) {
    const joined = value.map((item) => String(item).trim()).filter(Boolean).join("、");
    return joined === "" ? null : joined;
  }
  if (typeof value === "object") {
    return null;
  }
  const text = String(value).trim();
  return text === "" ? null : text;
}

/** Reads a frontmatter value as a list; strings are split on 、 , ， ; ；. */
export function fmList(data: FrontmatterData, key: string): string[] {
  const value = data[key];
  if (value === null || value === undefined) {
    return [];
  }
  const items = Array.isArray(value)
    ? value.map((item) => String(item))
    : typeof value === "object"
      ? []
      : String(value).split(/[、,，;；]/);
  return items.map((item) => item.trim()).filter((item) => item !== "");
}
