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
    const html = render("[官网](https://example.com) [文件](other.md) [邮件](mailto:someone@example.com)");
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('<span class="link--local" title="other.md">文件</span>');
    // Only http(s) leaves the app; other schemes are not clickable.
    expect(html).toContain('<span class="link--local" title="mailto:someone@example.com">邮件</span>');
  });

  it("does not render raw HTML from the source", () => {
    const html = render('<script>alert(1)</script><img src=x onerror="alert(1)">');
    // The source is shown as escaped text, never as elements.
    expect(html).not.toMatch(/<script|<img/);
    expect(html).toContain("&lt;script&gt;");
  });

  it("gives headings stable, unique ids", () => {
    const html = render("## 小节\n\n### Next Step!\n\n## 小节\n\n## 一、怎么做选择\n\n## 关于 [[底色]]");
    expect(html).toContain('<h2 id="小节">小节</h2>');
    expect(html).toContain('<h3 id="next-step">Next Step!</h3>');
    expect(html).toContain('<h2 id="小节-1">小节</h2>');
    expect(html).toContain('<h2 id="一怎么做选择">一、怎么做选择</h2>');
    expect(html).toMatch(/<h2 id="关于-底色">关于 <a /);
    // Rendering again yields the same ids.
    expect(render("## 小节\n\n## 小节")).toBe(render("## 小节\n\n## 小节"));
  });

  it("keeps in-page links on the page instead of changing the route", () => {
    const html = render("[见下文](#小节) 和 [[#小节|下一节]]\n\n## 小节");
    const anchors = html.match(/<a [^>]*>[^<]*<\/a>/g) ?? [];
    expect(anchors).toHaveLength(2);
    expect(anchors[0]).toMatch(/class="anchor-link"/);
    expect(anchors[0]).toContain(`href="#${encodeURIComponent("小节")}"`);
    expect(anchors[0]).toContain(">见下文</a>");
    expect(anchors[1]).toMatch(/class="wikilink anchor-link"/);
    expect(anchors[1]).toContain(">下一节</a>");
    // Wikilinks to other pages still navigate through the hash router.
    expect(render("[[写作即思考]]")).not.toContain("anchor-link");
  });
});
