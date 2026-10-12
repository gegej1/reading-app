// What the settings screen shows and saves (see src-tauri/src/settings.rs and
// src-tauri/src/app_config.rs), and the checks of its form. The backend
// checks everything again before it writes config.json.

import type { ShortcutStatus } from "../../app/capture";
import type { EngineKind, RunSettingsView } from "../../app/runs";

/** Where a remote points: a host, or a folder on this Mac. Never the URL itself. */
export interface RemoteInfo {
  name: string;
  host: string | null;
  localPath: string | null;
  /** The current branch syncs with this remote. */
  tracked: boolean;
}

export interface KbFolderCheck {
  path: string;
  ok: boolean;
  problem: string | null;
  hasRules: boolean;
  remote: RemoteInfo | null;
}

/** The iPhone bridge (src-tauri/src/bridge/mod.rs, BridgeStatus). */
export interface BridgeStatus {
  enabled: boolean;
  path: string | null;
  source: "env" | "config" | "default" | null;
  ready: boolean;
  problem: string | null;
  lastImport: string | null;
  shortcuts: string | null;
  waiting: number;
  /** Why nothing is imported right now (the import record cannot be read or written). */
  importProblem: string | null;
}

/** Where the bridge folder is, in the words of the Files app on the phone. */
export function bridgePlace(status: BridgeStatus): string {
  if (status.source === "default") return "iCloud 云盘 › 快捷指令 › 底色";
  return status.path ?? "没有文件夹";
}

export interface SettingsInfo {
  version: string;
  run: RunSettingsView;
  kb: { path: string; source: "env" | "config" | "default"; check: KbFolderCheck } | null;
  kbProblem: string | null;
  captureShortcut: ShortcutStatus | null;
  /** Launch at login; null when it cannot be told. */
  autostart: boolean | null;
  configProblem: string | null;
  /** Null when the app has no bridge (tests). */
  bridge: BridgeStatus | null;
}

export interface PickedFolder {
  path: string;
  check: KbFolderCheck;
}

/** The AI part of the form, as typed. */
export interface AiForm {
  engine: EngineKind;
  model: string;
  budget: string;
  timeout: string;
}

export type AiField = "model" | "budget" | "timeout";

/** What `settings_save_ai` gets. */
export interface AiSettings {
  engine: EngineKind;
  model: string | null;
  budgetUsd: number;
  runTimeoutMinutes: number;
}

export const MAX_BUDGET_USD = 1000;
export const MAX_TIMEOUT_MINUTES = 24 * 60;
const MAX_MODEL_CHARS = 120;

/** Model names the engines know, offered while typing (any other name works too). */
export const MODEL_SUGGESTIONS: Record<EngineKind, string[]> = {
  claude: ["sonnet", "opus", "haiku"],
  codex: ["gpt-5-codex", "gpt-5"],
};

export function formFromSettings(view: RunSettingsView): AiForm {
  return {
    engine: view.engine,
    model: view.model ?? "",
    budget: String(view.budgetUsd),
    timeout: String(Math.round(view.runTimeoutMinutes * 100) / 100),
  };
}

/** Checks the form; the values to save, or what is wrong with each field. */
export function checkAiForm(form: AiForm): { value: AiSettings; errors: null } | { value: null; errors: Partial<Record<AiField, string>> } {
  const errors: Partial<Record<AiField, string>> = {};
  const model = form.model.trim();
  if (model.length > MAX_MODEL_CHARS) errors.model = `模型名太长了（最多 ${MAX_MODEL_CHARS} 个字符）`;
  else if (/\s/.test(model)) errors.model = "模型名里不能有空格";
  const budget = Number(form.budget.trim());
  if (form.budget.trim() === "" || !Number.isFinite(budget) || budget <= 0 || budget > MAX_BUDGET_USD) {
    errors.budget = `要是大于 0、不超过 ${MAX_BUDGET_USD} 的美元数`;
  }
  const timeout = Number(form.timeout.trim());
  if (form.timeout.trim() === "" || !Number.isFinite(timeout) || timeout < 1 || timeout > MAX_TIMEOUT_MINUTES) {
    errors.timeout = `要是 1 到 ${MAX_TIMEOUT_MINUTES} 之间的分钟数`;
  }
  if (Object.keys(errors).length > 0) return { value: null, errors };
  return {
    value: { engine: form.engine, model: model === "" ? null : model, budgetUsd: budget, runTimeoutMinutes: timeout },
    errors: null,
  };
}

/** Where syncing goes, in a few words: `github.com`, or a folder on this Mac. */
export function remoteText(remote: RemoteInfo | null): string {
  if (!remote) return "没有远端：知识库只保存在这台 Mac 上";
  const where = remote.host ?? (remote.localPath ? `这台 Mac 上的文件夹 ${remote.localPath}` : "未知的地方");
  return remote.tracked ? `同步到 ${where}（${remote.name}）` : `远端 ${remote.name} 在 ${where}，但现在的分支还没连上它`;
}
