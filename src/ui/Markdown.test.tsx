import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { Markdown, type LinkResolver } from "./Markdown";

const pages: Record<string, string> = {
  写作即思考: "wiki/概念/写作即思考.md",
  底色: "self/底色.md",
};

const resolve: LinkResolver = (target) => {
  const path = pages[target];
  if (!path) return null;
  return { href: `#/page/${encodeURIComponent(target)}`, title: path };
};

const render = (md: string) => renderToStaticMarkup(<Markdown resolve={resolve}>{md}</Markdown>);

describe("Markdown", () => {
  it("turns wikilinks into in-app links with their labels", () => {
    const html = render("见 [[写作即思考]] 和 [[写作即思考#要点|要点]]。");
    const links = html.match(/<a [^>]*>[^<]*<\/a>/g) ?? [];
    expect(links).toHaveLength(2);
    for (const link of links) {
      expect(link).toContain(`href="#/page/${encodeURIComponent("写作即思考")}"`);
      expect(link).toContain('class="wikilink"');
      expect(link).toContain('title="wiki/概念/写作即思考.md"');
      expect(link).toContain('data-wikilink="写作即思考"');
    }
    expect(links[0]).toContain(">写作即思考</a>");
    expect(links[1]).toContain(">要点</a>");
  });

  it("marks broken links", () => {
    const html = render("[[不存在]]");
    expect(html).toContain(
      '<span class="wikilink wikilink--broken" title="找不到页面：不存在" data-wikilink="不存在">不存在</span>',
    );
  });

  it("leaves links in code alone", () => {
    const html = render("`[[写作即思考]]`\n\n```\n[[底色]]\n```");
    expect(html).not.toContain("wikilink");
    expect(html).toContain("<code>[[写作即思考]]</code>");
  });

  it("renders links inside tables, lists and blockquotes", () => {
    const html = render(
      "| 页 | 说明 |\n|---|---|\n| [[底色]] | 骨架 |\n\n- [[底色]]\n\n> 引用 [[底色]]",
    );
    expect(html).toContain('<div class="table-wrap"><table>');
    expect(html.match(/class="wikilink"/g)?.length).toBe(3);
  });

  it("renders task lists as read-only checkboxes", () => {
    const html = render("- [ ] 未做\n- [x] 已做");
    expect(html).toContain('type="checkbox"');
    expect(html.match(/disabled=""/g)?.length).toBe(2);
    expect(html).toContain('aria-label="已勾选"');
  });

  it("opens external links outside the app and neutralises local file links", () => {
    const html = render("[官网](https://example.com) [文件](other.md)");
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noreferrer noopener"');
    expect(html).toContain('<span class="link--local" title="other.md">文件</span>');
  });

  it("does not render raw HTML from the source", () => {
    const html = render('<script>alert(1)</script><img src=x onerror="alert(1)">');
    // The source is shown as escaped text, never as elements.
    expect(html).not.toMatch(/<script|<img/);
    expect(html).toContain("&lt;script&gt;");
  });
});
