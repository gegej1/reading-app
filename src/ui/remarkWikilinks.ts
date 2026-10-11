import type { Parent, PhrasingContent, Root, RootContent, Text } from "mdast";
import { parseWikilinkInner } from "../domain/wikilinks";

/** Where a wikilink points; null marks a broken link. */
export interface WikilinkTarget {
  href: string;
  /** Tooltip, e.g. the target page's path. */
  title?: string;
}

export interface RemarkWikilinksOptions {
  resolve(target: string): WikilinkTarget | null;
}

const WIKILINK = /(!?)\[\[([^[\]\n]+?)\]\]/g;

/** Node types whose text must never become links. */
const SKIP = new Set(["link", "linkReference", "inlineCode", "code", "html", "definition"]);

function label(inner: string): string {
  const { target, heading, alias } = parseWikilinkInner(inner);
  if (alias) return alias;
  if (heading) return target ? `${target} › ${heading}` : heading;
  return target;
}

function linkNode(raw: string, inner: string, embed: boolean, options: RemarkWikilinksOptions) {
  const { target } = parseWikilinkInner(inner);
  const resolved = target ? options.resolve(target) : null;
  const text = label(inner) || raw;
  const className = ["wikilink"];
  if (embed) className.push("wikilink--embed");
  if (!resolved) className.push("wikilink--broken");
  // mdast-util-to-hast turns unknown nodes with hName/hProperties into elements.
  const node = {
    type: "wikiLink",
    value: text,
    data: {
      hName: resolved ? "a" : "span",
      hProperties: resolved
        ? { href: resolved.href, className, title: resolved.title ?? target, "data-wikilink": target }
        : { className, title: `找不到页面：${target || raw}`, "data-wikilink": target },
      hChildren: [{ type: "text", value: text }],
    },
  };
  return node as unknown as PhrasingContent;
}

function splitText(node: Text, options: RemarkWikilinksOptions): PhrasingContent[] | null {
  const value = node.value;
  WIKILINK.lastIndex = 0;
  if (!WIKILINK.test(value)) return null;
  WIKILINK.lastIndex = 0;

  const parts: PhrasingContent[] = [];
  let last = 0;
  for (const match of value.matchAll(WIKILINK)) {
    const index = match.index ?? 0;
    const inner = match[2] ?? "";
    if (parseWikilinkInner(inner).target === "" && parseWikilinkInner(inner).heading === null) {
      continue;
    }
    if (index > last) parts.push({ type: "text", value: value.slice(last, index) });
    parts.push(linkNode(match[0], inner, match[1] === "!", options));
    last = index + match[0].length;
  }
  if (parts.length === 0) return null;
  if (last < value.length) parts.push({ type: "text", value: value.slice(last) });
  return parts;
}

function walk(parent: Parent, options: RemarkWikilinksOptions): void {
  const next: RootContent[] = [];
  let changed = false;
  for (const child of parent.children) {
    if (child.type === "text") {
      const replaced = splitText(child, options);
      if (replaced) {
        next.push(...replaced);
        changed = true;
        continue;
      }
    } else if (!SKIP.has(child.type) && "children" in child) {
      walk(child as Parent, options);
    }
    next.push(child);
  }
  if (changed) parent.children = next as Parent["children"];
}

/**
 * Remark plugin: turns `[[name]]`, `[[name|alias]]` and `[[name#heading]]` in
 * text into links. Code spans and code blocks are separate node types, so
 * links inside backticks stay literal. Unresolved targets become
 * `span.wikilink--broken`.
 */
export function remarkWikilinks(options: RemarkWikilinksOptions) {
  return (tree: Root) => {
    walk(tree, options);
  };
}
