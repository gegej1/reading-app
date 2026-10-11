import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SyncIndicatorView } from "./SyncIndicator";
import type { SyncStatus } from "./sync";

const NOW = new Date(2026, 9, 12, 14, 30).getTime();

const status = (partial: Partial<SyncStatus> = {}): SyncStatus => ({
  running: false,
  repo: { branch: "main", upstream: "origin/main", ahead: 0, behind: 0, dirty: false },
  last: {
    at: NOW - 120_000,
    reason: "periodic",
    state: "synced",
    detail: null,
    paths: [],
    pulled: 0,
    pushed: 0,
    changed: false,
    backup: null,
  },
  busy: null,
  ...partial,
});

const render = (value: SyncStatus | null, requesting = false) =>
  renderToStaticMarkup(<SyncIndicatorView status={value} requesting={requesting} onSync={() => undefined} now={NOW} />);

const text = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();

describe("SyncIndicatorView", () => {
  it("shows the five states the sidebar knows", () => {
    expect(text(render(status()))).toMatch(/^已同步 已同步 · 2 分钟前/);
    const ahead = render(status({ repo: { branch: "main", upstream: "origin/main", ahead: 2, behind: 0, dirty: false } }));
    expect(ahead).toContain('<span class="sync__count">↑2</span><span class="sync__label">待推送</span>');
    const behind = render(status({ repo: { branch: "main", upstream: "origin/main", ahead: 0, behind: 4, dirty: false } }));
    expect(behind).toContain('<span class="sync__count">↓4</span><span class="sync__label">待拉取</span>');
    expect(render(status({ last: { ...status().last!, state: "offline" } }))).toContain(
      '<span class="sync__label">离线</span>',
    );
    const conflict = render(
      status({ last: { ...status().last!, state: "conflict", paths: ["self/底色.md", "wiki/概念/甲.md"] } }),
    );
    expect(conflict).toContain('class="sync is-attention"');
    expect(conflict).toContain('<span class="sync__label">需要处理</span>');
    expect(text(conflict)).toContain("git pull --rebase");
    expect(conflict).toContain("<li><code>self/底色.md</code></li><li><code>wiki/概念/甲.md</code></li>");
  });

  it("spins while syncing and is a button that says what a click does", () => {
    const running = render(status({ running: true }));
    expect(running).toContain('class="sync is-quiet is-running"');
    expect(running).toContain("disabled");
    expect(running).toContain('<span class="sync__label">同步中</span>');
    expect(render(status(), true)).toContain("disabled");
    expect(render(status())).toMatch(/aria-label="已同步 · 2 分钟前。点一下立即同步"/);
  });

  it("waits quietly before the first status arrives", () => {
    expect(text(render(null))).toMatch(/^同步 正在查看同步状态/);
  });
});
