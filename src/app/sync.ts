// What the sidebar shows about syncing with the remote: the status the
// backend reports (`kb_status`, `kb_sync`, the `sync-status` event; see
// src-tauri/src/sync_service.rs) turned into a short label and, where a
// person has to step in, a plain explanation of what to do.

import { useCallback, useEffect, useRef, useState } from "react";
import type { Desktop } from "./desktop";

export type SyncState =
  | "synced"
  | "offline"
  | "busy"
  | "remoteMoved"
  | "dirty"
  | "conflict"
  | "noUpstream"
  | "notRepo"
  | "detached"
  | "inProgress"
  | "failed";

export interface RepoStatus {
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  dirty: boolean;
}

export interface LastSync {
  /** When it ended, milliseconds since the epoch. */
  at: number;
  reason: string;
  state: SyncState;
  detail: string | null;
  paths: string[];
  pulled: number;
  pushed: number;
  changed: boolean;
  backup: string | null;
}

export interface SyncStatus {
  running: boolean;
  repo: RepoStatus | null;
  last: LastSync | null;
  /** Another operation holds the knowledge base (what it is doing). */
  busy: string | null;
}

export type SyncTone = "ok" | "pending" | "offline" | "attention" | "quiet";

export interface SyncView {
  /** Short label for the sidebar: 已同步, 待推送, 离线, 需要处理 … */
  label: string;
  /** `↑2` or `↓3`, shown upright next to the label. */
  count: string | null;
  tone: SyncTone;
  running: boolean;
  /** One line about the state. */
  title: string;
  /** What to do, when a person has to step in. */
  explain: string | null;
  /** Files concerned (conflicts, local changes in the way). */
  paths: string[];
}

const ATTENTION: Partial<Record<SyncState, { title: string; explain: (detail: string | null) => string }>> = {
  dirty: {
    title: "有没提交的改动挡住了同步",
    explain: () =>
      "知识库文件夹里有改过但还没提交的文件，和远端的新内容碰到了一起。为了不覆盖它们，这次没有同步。先把这些改动提交（或者撤销），再点这里重新同步。",
  },
  conflict: {
    title: "本机和远端改了同一处",
    explain: () =>
      "两边改了同一个地方，没法自动合并。本机的内容一点没动，也还没有推送。需要手动合并：在终端里进入知识库文件夹，运行 git pull --rebase，按提示改好冲突的文件后再 git push。",
  },
  noUpstream: {
    title: "知识库还没连上远端",
    explain: () =>
      "现在的分支没有对应的远端分支，改动只保存在本机。在终端里进入知识库文件夹，运行一次 git push -u origin main（远端名字和分支名按实际情况改），以后就会自动同步。",
  },
  notRepo: {
    title: "知识库文件夹不是 git 仓库",
    explain: (detail) =>
      `没法同步：知识库文件夹不是一个 git 仓库的根目录。${detail ? `（${detail}）` : ""}`,
  },
  detached: {
    title: "知识库没有停在分支上",
    explain: () => "知识库现在处在“detached HEAD”状态。在终端里切回分支（比如 git switch main）后再同步。",
  },
  inProgress: {
    title: "有一个没做完的 git 操作",
    explain: (detail) =>
      `知识库里有一个没做完的 git ${detail ?? "操作"}。先在终端里完成它，或者取消它（比如 git ${detail === "merge" ? "merge" : "rebase"} --abort），再同步。`,
  },
  failed: {
    title: "同步出错了",
    explain: (detail) => `同步时出了意外，知识库没有被改动。${detail ? `git 的说法：${detail}` : ""}`,
  },
};

/** `刚刚`, `5 分钟前`, `14:03` today, else `10 月 11 日 14:03`. */
export function formatSince(at: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 60) return "刚刚";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  const then = new Date(at);
  const time = `${String(then.getHours()).padStart(2, "0")}:${String(then.getMinutes()).padStart(2, "0")}`;
  const today = new Date(now);
  const sameDay =
    then.getFullYear() === today.getFullYear() && then.getMonth() === today.getMonth() && then.getDate() === today.getDate();
  return sameDay ? time : `${then.getMonth() + 1} 月 ${then.getDate()} 日 ${time}`;
}

/** The sidebar's view of a status (null: not known yet). */
export function describeSync(status: SyncStatus | null, now: number = Date.now()): SyncView {
  const view = (partial: Partial<SyncView> & Pick<SyncView, "label" | "tone" | "title">): SyncView => ({
    count: null,
    running: false,
    explain: null,
    paths: [],
    ...partial,
  });
  if (!status) return view({ label: "同步", tone: "quiet", title: "正在查看同步状态" });
  if (status.running) return view({ label: "同步中", tone: "quiet", running: true, title: "正在和远端同步…" });

  const last = status.last;
  const attention = last ? ATTENTION[last.state] : undefined;
  if (last && attention) {
    return view({
      label: "需要处理",
      tone: "attention",
      title: attention.title,
      explain: attention.explain(last.detail),
      paths: last.paths,
    });
  }

  const ahead = status.repo?.ahead ?? 0;
  const behind = status.repo?.behind ?? 0;
  if (last?.state === "offline") {
    return view({
      label: "离线",
      count: ahead > 0 ? `↑${ahead}` : null,
      tone: "offline",
      title: ahead > 0 ? `连不上远端，${ahead} 个提交先留在本机，联网后会自动推送` : "连不上远端，联网后会自动同步",
    });
  }
  if (last?.state === "busy") {
    return view({
      label: "稍后同步",
      tone: "quiet",
      title: `知识库正在${last.detail ?? "忙"}，稍后会自动再同步`,
    });
  }
  if (ahead > 0) {
    return view({ label: "待推送", count: `↑${ahead}`, tone: "pending", title: `有 ${ahead} 个提交还没推送到远端` });
  }
  if (behind > 0) {
    return view({ label: "待拉取", count: `↓${behind}`, tone: "pending", title: `远端有 ${behind} 个新提交还没拉取` });
  }
  if (!last) return view({ label: "同步", tone: "quiet", title: "还没同步过，点一下现在同步" });
  return view({ label: "已同步", tone: "ok", title: `已同步 · ${formatSince(last.at, now)}` });
}

/**
 * The live sync status on the desktop: asks once, then follows `sync-status`.
 * `syncNow` runs a sync and resolves when it is done.
 */
export function useSyncStatus(bridge: Desktop, enabled: boolean) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [requesting, setRequesting] = useState(false);
  const heardEvent = useRef(false);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const stop = bridge.listen<SyncStatus>("sync-status", (next) => {
      heardEvent.current = true;
      setStatus(next);
    });
    bridge
      .call<SyncStatus>("kb_status")
      .then((first) => {
        // An event that arrived meanwhile is newer.
        if (alive && !heardEvent.current) setStatus(first);
      })
      .catch((error: unknown) => bridge.log("warn", `kb_status failed: ${error instanceof Error ? error.message : String(error)}`));
    return () => {
      alive = false;
      stop();
    };
  }, [bridge, enabled]);

  const syncNow = useCallback(async () => {
    setRequesting(true);
    try {
      setStatus(await bridge.call<SyncStatus>("kb_sync"));
    } catch (error) {
      bridge.log("warn", `kb_sync failed: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setRequesting(false);
    }
  }, [bridge]);

  return { status, syncNow, requesting };
}
