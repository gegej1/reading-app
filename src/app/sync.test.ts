import { describe, expect, it } from "vitest";
import { describeSync, formatSince, type LastSync, type RepoStatus, type SyncState, type SyncStatus } from "./sync";

const NOW = new Date(2026, 9, 12, 14, 30).getTime();

const repo = (ahead = 0, behind = 0): RepoStatus => ({
  branch: "main",
  upstream: "origin/main",
  ahead,
  behind,
  dirty: false,
});

const last = (state: SyncState, extra: Partial<LastSync> = {}): LastSync => ({
  at: NOW - 30_000,
  reason: "periodic",
  state,
  detail: null,
  paths: [],
  pulled: 0,
  pushed: 0,
  changed: false,
  backup: null,
  ...extra,
});

const status = (partial: Partial<SyncStatus> = {}): SyncStatus => ({
  running: false,
  repo: repo(),
  last: last("synced"),
  busy: null,
  ...partial,
});

describe("describeSync", () => {
  it("says 已同步 with the time of the last sync", () => {
    const view = describeSync(status(), NOW);
    expect(view).toMatchObject({ label: "已同步", tone: "ok", count: null, explain: null });
    expect(view.title).toBe("已同步 · 刚刚");
  });

  it("counts commits to push and to pull", () => {
    expect(describeSync(status({ repo: repo(2, 0) }), NOW)).toMatchObject({
      label: "待推送",
      count: "↑2",
      tone: "pending",
    });
    expect(describeSync(status({ repo: repo(0, 3) }), NOW)).toMatchObject({
      label: "待拉取",
      count: "↓3",
      tone: "pending",
    });
    // Pushing first: that is what the app owes the remote.
    expect(describeSync(status({ repo: repo(1, 1) }), NOW).count).toBe("↑1");
  });

  it("says 离线 and keeps counting what waits to be pushed", () => {
    expect(describeSync(status({ last: last("offline") }), NOW)).toMatchObject({ label: "离线", tone: "offline", count: null });
    const waiting = describeSync(status({ last: last("offline"), repo: repo(2) }), NOW);
    expect(waiting).toMatchObject({ label: "离线", count: "↑2" });
    expect(waiting.title).toContain("联网后会自动推送");
  });

  it("asks for a person, in plain words, when something is in the way", () => {
    const dirty = describeSync(status({ last: last("dirty", { paths: ["self/底色.md"] }) }), NOW);
    expect(dirty).toMatchObject({ label: "需要处理", tone: "attention", paths: ["self/底色.md"] });
    expect(dirty.explain).toContain("提交");

    const conflict = describeSync(status({ last: last("conflict", { paths: ["wiki/概念/甲.md"] }) }), NOW);
    expect(conflict.label).toBe("需要处理");
    expect(conflict.explain).toContain("git pull --rebase");
    expect(conflict.explain).toContain("本机的内容一点没动");

    const noUpstream = describeSync(status({ last: last("noUpstream", { detail: "main" }) }), NOW);
    expect(noUpstream.explain).toContain("git push -u");

    for (const state of ["notRepo", "detached", "inProgress", "failed"] as const) {
      const view = describeSync(status({ last: last(state, { detail: "rebase" }) }), NOW);
      expect(view.tone, state).toBe("attention");
      expect(view.explain, state).toBeTruthy();
    }
    expect(describeSync(status({ last: last("inProgress", { detail: "merge" }) }), NOW).explain).toContain(
      "git merge --abort",
    );
  });

  it("shows a running sync, a busy knowledge base and the time before the first sync", () => {
    expect(describeSync(status({ running: true }), NOW)).toMatchObject({ label: "同步中", running: true });
    expect(describeSync(status({ last: last("busy", { detail: "写入" }) }), NOW)).toMatchObject({
      label: "稍后同步",
      title: "知识库正在写入，稍后会自动再同步",
    });
    expect(describeSync(status({ last: null }), NOW).label).toBe("同步");
    expect(describeSync(status({ last: null, repo: repo(1) }), NOW).label).toBe("待推送");
    expect(describeSync(null, NOW)).toMatchObject({ label: "同步", tone: "quiet" });
    // A transient race with the remote: show what is waiting.
    expect(describeSync(status({ last: last("remoteMoved"), repo: repo(1) }), NOW).label).toBe("待推送");
  });

  it("never mentions internal names", () => {
    const states: SyncState[] = ["synced", "offline", "busy", "remoteMoved", "dirty", "conflict", "noUpstream", "notRepo", "detached", "inProgress", "failed"];
    for (const state of states) {
      const view = describeSync(status({ last: last(state) }), NOW);
      const text = `${view.label} ${view.title} ${view.explain ?? ""}`;
      expect(text, state).not.toMatch(/(?<![A-Za-z])M[0-9]|sync_|SyncState|kb_/);
    }
  });
});

describe("formatSince", () => {
  it("says how long ago, in Chinese", () => {
    expect(formatSince(NOW - 5_000, NOW)).toBe("刚刚");
    expect(formatSince(NOW - 5 * 60_000, NOW)).toBe("5 分钟前");
    expect(formatSince(new Date(2026, 9, 12, 9, 5).getTime(), NOW)).toBe("09:05");
    expect(formatSince(new Date(2026, 9, 11, 23, 59).getTime(), NOW)).toBe("10 月 11 日 23:59");
    expect(formatSince(NOW + 10_000, NOW)).toBe("刚刚");
  });
});
