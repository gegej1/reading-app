import type { Root } from "mdast";
import { createSlugger } from "./anchors";

interface AnyNode {
  type: string;
  value?: unknown;
  children?: AnyNode[];
  data?: { hProperties?: Record<string, unknown> } & Record<string, unknown>;
}

/** Visible text of a node: text, inline code and wikilink labels (raw HTML is not rendered). */
function textOf(node: AnyNode): string {
  if (node.type === "html") return "";
  if (typeof node.value === "string") return node.value;
  return (node.children ?? []).map(textOf).join("");
}

/**
 * Remark plugin: gives every heading a stable id (see `headingSlug`), numbered
 * in document order when headings repeat, so in-page links can find them.
 * Run it after the wikilink plugin so wikilink labels count as heading text.
 */
export function remarkHeadingIds() {
  return (tree: Root) => {
    const slug = createSlugger();
    const visit = (node: AnyNode) => {
      if (node.type === "heading") {
        const data = (node.data ??= {});
        data.hProperties = { ...data.hProperties, id: slug(textOf(node)) };
        return;
      }
      node.children?.forEach(visit);
    };
    visit(tree as unknown as AnyNode);
  };
}
