import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useKb } from "../../app/KbContext";
import { desktop, type Desktop } from "../../app/desktop";
import { useRun } from "../../app/RunContext";
import type { ShortcutStatus } from "../../app/capture";
import type { EngineKind } from "../../app/runs";
import { ENGINE_LABEL, TASK_LABEL } from "../../app/runView";
import { cx, EmptyState, PageHeader } from "../../ui/components";
import {
  bridgePlace,
  checkAiForm,
  formFromSettings,
  MODEL_SUGGESTIONS,
  remoteText,
  type AiField,
  type AiForm,
  type BridgeStatus,
  type KbFolderCheck,
  type PickedFolder,
  type SettingsInfo,
} from "./settings";
import { modifierLabel, onlyModifiers, recordShortcut } from "./shortcutKeys";
import "../../styles/settings.css";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const DEFAULT_SHORTCUT = "Ctrl+Alt+N";

const SOURCE_TEXT: Record<"env" | "config" | "default", string> = {
  env: "由启动 App 时的环境变量指定",
  config: "在设置里选的",
  default: "默认位置",
};

function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="settings-section" aria-label={title}>
      <header className="settings-section__head">
        <h2 className="settings-section__title">{title}</h2>
        {aside ? <span className="settings-section__aside">{aside}</span> : null}
      </header>
      {children}
    </section>
  );
}

function FieldError({ text }: { text: string | undefined }) {
  return text ? (
    <p className="field__error" role="alert">
      {text}
    </p>
  ) : null;
}

/** The engine, model, budget and time limit of the runs. */
function AiSettingsForm({ info, bridge, onSaved }: { info: SettingsInfo; bridge: Desktop; onSaved(next: SettingsInfo): void }) {
  const { notify } = useKb();
  const [form, setForm] = useState<AiForm>(() => formFromSettings(info.run));
  const [errors, setErrors] = useState<Partial<Record<AiField, string>>>({});
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const saved = formFromSettings(info.run);
  const dirty = JSON.stringify(form) !== JSON.stringify(saved);

  const set = (patch: Partial<AiForm>) => {
    setForm((current) => ({ ...current, ...patch }));
    setErrors({});
    setFailure(null);
  };

  const save = async () => {
    const checked = checkAiForm(form);
    if (!checked.value) {
      setErrors(checked.errors);
      return;
    }
    setSaving(true);
    setFailure(null);
    try {
      const next = await bridge.call<SettingsInfo>("settings_save_ai", { settings: checked.value });
      onSaved(next);
      setForm(formFromSettings(next.run));
      notify("已保存，下一次运行开始用新的设置");
    } catch (error) {
      setFailure(errorText(error));
    } finally {
      setSaving(false);
    }
  };

  const program = (engine: EngineKind) => (engine === "claude" ? info.run.claudeProgram : info.run.codexProgram);

  return (
    <form
      className="settings-form"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <fieldset className="engine-cards">
        <legend className="field__label">引擎</legend>
        {(["claude", "codex"] as const).map((engine) => (
          <label key={engine} className={cx("engine-card", form.engine === engine && "is-chosen")}>
            <input
              type="radio"
              name="engine"
              value={engine}
              checked={form.engine === engine}
              onChange={() => set({ engine, model: engine === saved.engine ? saved.model : "" })}
            />
            <span className="engine-card__name">
              {ENGINE_LABEL[engine]}
              {engine === "claude" ? " Code" : ""}
            </span>
            <span className={cx("engine-card__program", !program(engine) && "is-missing")}>
              {program(engine) ? `找到了：${program(engine)}` : `没在这台 Mac 上找到 ${engine}`}
            </span>
          </label>
        ))}
      </fieldset>

      <label className="field">
        <span className="field__label">模型</span>
        <input
          className="field__input"
          list={`models-${form.engine}`}
          value={form.model}
          onChange={(event) => set({ model: event.target.value })}
          placeholder={`留空：跟随你自己的 ${ENGINE_LABEL[form.engine]} 设置`}
          spellCheck={false}
          autoComplete="off"
        />
        <datalist id={`models-${form.engine}`}>
          {MODEL_SUGGESTIONS[form.engine].map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
        <span className="field__hint">可以写别名（如 sonnet）或完整的模型名。只对上面选的引擎有效。</span>
        <FieldError text={errors.model} />
      </label>

      <div className="field-row">
        <label className="field">
          <span className="field__label">每次运行的花费上限</span>
          <span className="field__affix">
            <span className="field__prefix">$</span>
            <input
              className="field__input field__input--number"
              inputMode="decimal"
              value={form.budget}
              onChange={(event) => set({ budget: event.target.value })}
            />
          </span>
          <span className="field__hint">美元，只对 Claude 有效（Codex 不支持上限）。</span>
          <FieldError text={errors.budget} />
        </label>
        <label className="field">
          <span className="field__label">运行时限</span>
          <span className="field__affix">
            <input
              className="field__input field__input--number"
              inputMode="numeric"
              value={form.timeout}
              onChange={(event) => set({ timeout: event.target.value })}
            />
            <span className="field__suffix">分钟</span>
          </span>
          <span className="field__hint">超过这么久，AI 会被停下。</span>
          <FieldError text={errors.timeout} />
        </label>
      </div>

      {info.run.problems.map((problem) => (
        <p key={problem} className="settings-note" role="note">
          {problem}
        </p>
      ))}
      {failure ? (
        <p className="field__error" role="alert">
          {failure}
        </p>
      ) : null}
      <div className="settings-form__actions">
        <button type="submit" className="btn btn--seal" disabled={!dirty || saving || info.configProblem !== null}>
          {saving ? "正在保存…" : "保存"}
        </button>
        {dirty ? (
          <button type="button" className="btn btn--quiet" onClick={() => set(saved)} disabled={saving}>
            还原
          </button>
        ) : (
          <span className="settings-form__saved">已经是现在的设置</span>
        )}
      </div>
    </form>
  );
}

function CheckLine({ check }: { check: KbFolderCheck }) {
  if (!check.ok) {
    return (
      <p className="kb-check is-bad" role="alert">
        {check.problem}
      </p>
    );
  }
  return (
    <div className="kb-check is-ok">
      <p>是 git 仓库的根目录 · {remoteText(check.remote)}</p>
      {check.hasRules ? null : <p className="kb-check__warn">里面没有 AGENTS.md：入库和落裁决需要它，没有它不会开始。</p>}
    </div>
  );
}

/** Where the knowledge base is, and choosing another folder. */
function KbSettings({ info, bridge, onSaved }: { info: SettingsInfo; bridge: Desktop; onSaved(next: SettingsInfo): void }) {
  const run = useRun();
  const [picked, setPicked] = useState<PickedFolder | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const blocked = run.activeTask ? `正在${TASK_LABEL[run.activeTask]}，结束后再换知识库` : null;

  const pick = async () => {
    setBusy(true);
    setFailure(null);
    try {
      const chosen = await bridge.call<PickedFolder | null>("settings_pick_kb");
      if (chosen) setPicked(chosen);
    } catch (error) {
      setFailure(errorText(error));
    } finally {
      setBusy(false);
    }
  };

  const use = async () => {
    if (!picked) return;
    setBusy(true);
    setFailure(null);
    try {
      const next = await bridge.call<SettingsInfo>("settings_set_kb", { path: picked.path });
      onSaved(next);
      setPicked(null);
      // Everything shown comes from the old folder: start over on the new one.
      window.location.reload();
    } catch (error) {
      setFailure(errorText(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="kb-settings">
      {info.kb ? (
        <>
          <p className="kb-path">
            <code>{info.kb.path}</code>
            <span className="kb-path__source">{SOURCE_TEXT[info.kb.source]}</span>
          </p>
          <CheckLine check={info.kb.check} />
          {info.kb.source === "env" ? (
            <p className="settings-note" role="note">
              现在的文件夹是启动 App 时用环境变量指定的；在这里换的文件夹会存进设置，但要等不再用那个环境变量启动时才生效。
            </p>
          ) : null}
        </>
      ) : (
        <p className="kb-check is-bad">{info.kbProblem}</p>
      )}
      <div className="settings-form__actions">
        <button
          type="button"
          className="btn"
          onClick={() => void pick()}
          disabled={busy || blocked !== null || info.configProblem !== null}
          title={blocked ?? undefined}
        >
          更换…
        </button>
        {blocked ? <span className="settings-form__saved">{blocked}</span> : null}
      </div>
      {picked ? (
        <div className={cx("kb-candidate", picked.check.ok ? "is-ok" : "is-bad")}>
          <p className="kb-candidate__label">新选的文件夹</p>
          <p className="kb-path">
            <code>{picked.path}</code>
          </p>
          <CheckLine check={picked.check} />
          <div className="settings-form__actions">
            <button
              type="button"
              className="btn btn--seal"
              onClick={() => void use()}
              disabled={!picked.check.ok || busy || blocked !== null}
            >
              用这个文件夹
            </button>
            <button type="button" className="btn btn--quiet" onClick={() => setPicked(null)} disabled={busy}>
              算了
            </button>
          </div>
        </div>
      ) : null}
      {failure ? (
        <p className="field__error" role="alert">
          {failure}
        </p>
      ) : null}
    </div>
  );
}

/** The capture shortcut: a key recorder that registers the new one at once. */
function ShortcutSettings({ status, bridge, onChanged }: { status: ShortcutStatus | null; bridge: Desktop; onChanged(next: ShortcutStatus): void }) {
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);

  const apply = useCallback(
    async (setting: string) => {
      setBusy(true);
      setFailure(null);
      setDone(null);
      try {
        const next = await bridge.call<ShortcutStatus>("settings_set_shortcut", { setting });
        onChanged(next);
        setDone(`已换成 ${next.label}，现在就能用`);
      } catch (error) {
        setFailure(errorText(error));
      } finally {
        setBusy(false);
      }
    },
    [bridge, onChanged],
  );

  useEffect(() => {
    if (!recording) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.isComposing) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        setRecording(false);
        setHeld("");
        return;
      }
      if (onlyModifiers(event)) {
        setHeld(modifierLabel(event));
        return;
      }
      const recorded = recordShortcut(event);
      if (!recorded) return;
      setRecording(false);
      setHeld("");
      if (recorded.problem) {
        setFailure(recorded.problem);
        return;
      }
      void apply(recorded.setting);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (onlyModifiers(event)) setHeld(modifierLabel(event));
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [recording, apply]);

  const current = status?.label || status?.setting || "未设置";
  return (
    <div className="shortcut-settings">
      <div className={cx("keycap-well", recording && "is-recording")} aria-live="polite">
        {recording ? (
          <span className="keycap-well__prompt">{held ? `${held}…` : "按下新的快捷键…"}</span>
        ) : (
          <kbd className="keycap">{current}</kbd>
        )}
        {!recording && status && !status.registered ? <span className="keycap-well__bad">现在不可用</span> : null}
      </div>
      {recording ? (
        <p className="field__hint">要带 ⌃、⌥ 或 ⌘，最好两个以上，比如 ⌃⌥N。按 Esc 不换了。</p>
      ) : status?.error ? (
        <p className="field__error">{status.error}</p>
      ) : null}
      <div className="settings-form__actions">
        <button
          type="button"
          className="btn"
          onClick={() => {
            setFailure(null);
            setDone(null);
            setRecording(true);
          }}
          disabled={recording || busy}
        >
          {busy ? "正在设置…" : "录新的快捷键"}
        </button>
        {status?.setting !== DEFAULT_SHORTCUT || !status?.registered ? (
          <button type="button" className="btn btn--quiet" onClick={() => void apply(DEFAULT_SHORTCUT)} disabled={recording || busy}>
            恢复默认 ⌃⌥N
          </button>
        ) : null}
      </div>
      {failure ? (
        <p className="field__error" role="alert">
          {failure}
        </p>
      ) : null}
      {done ? <p className="settings-done">{done}</p> : null}
    </div>
  );
}

/** Launch at login: off unless the user turns it on. */
function AutostartSwitch({ enabled, bridge, onChanged }: { enabled: boolean | null; bridge: Desktop; onChanged(next: boolean): void }) {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const toggle = async () => {
    setBusy(true);
    setFailure(null);
    try {
      onChanged(await bridge.call<boolean>("settings_set_autostart", { enabled: !enabled }));
    } catch (error) {
      setFailure(errorText(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="autostart">
      <label className={cx("switch", enabled && "is-on")}>
        <input type="checkbox" role="switch" checked={enabled ?? false} onChange={() => void toggle()} disabled={busy || enabled === null} />
        <span className="switch__track" aria-hidden="true">
          <span className="switch__thumb" />
        </span>
        <span className="switch__label">登录这台 Mac 时自动打开底色</span>
      </label>
      <p className="field__hint">
        {enabled === null ? "没法读出现在的状态。" : enabled ? "已打开：登录后底色会在菜单栏里待命。" : "默认关闭。"}
      </p>
      {failure ? (
        <p className="field__error" role="alert">
          {failure}
        </p>
      ) : null}
    </div>
  );
}

/** The iPhone bridge through iCloud: on or off, where its folder is, how it goes. */
function BridgeSettings({
  status,
  bridge,
  onSaved,
  onOpen,
}: {
  status: BridgeStatus;
  bridge: Desktop;
  onSaved(next: SettingsInfo): void;
  onOpen(): void;
}) {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const toggle = async () => {
    setBusy(true);
    setFailure(null);
    try {
      onSaved(await bridge.call<SettingsInfo>("settings_set_bridge", { enabled: !status.enabled }));
    } catch (error) {
      setFailure(errorText(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="autostart">
      <label className={cx("switch", status.enabled && "is-on")}>
        <input type="checkbox" role="switch" checked={status.enabled} onChange={() => void toggle()} disabled={busy} />
        <span className="switch__track" aria-hidden="true">
          <span className="switch__thumb" />
        </span>
        <span className="switch__label">通过 iCloud 云盘和 iPhone 互通</span>
      </label>
      <p className="field__hint">
        手机上用「快捷指令」记一笔、回答待裁决，这台 Mac 收进知识库；底色、判例、待裁决、待抽查和最近一次入库做成手机页面放进 iCloud。原料、知识页和体系不会离开这台 Mac。
      </p>
      {status.enabled ? (
        <dl className="settings-facts">
          <dt>文件夹</dt>
          <dd>
            {bridgePlace(status)}
            {status.source === "env" ? "（由环境变量 DISE_BRIDGE_DIR 指定）" : null}
          </dd>
          <dt>状态</dt>
          <dd>
            {status.problem
              ? status.problem
              : status.ready
                ? status.waiting > 0
                  ? `正在看着；有 ${status.waiting} 个文件在等（iCloud 还在同步，或者知识库正忙）`
                  : "正在看着，手机上的新文件会自动收进来"
                : "正在准备…"}
          </dd>
          {status.importProblem ? (
            <>
              <dt>注意</dt>
              <dd className="field__error" role="alert">
                {status.importProblem}
              </dd>
            </>
          ) : null}
          {status.lastImport ? (
            <>
              <dt>最近</dt>
              <dd>{status.lastImport}</dd>
            </>
          ) : null}
          {status.shortcuts ? (
            <>
              <dt>快捷指令</dt>
              <dd>{status.shortcuts}</dd>
            </>
          ) : null}
        </dl>
      ) : null}
      {status.enabled && status.ready ? (
        <div className="settings-form__actions">
          <button type="button" className="btn" onClick={onOpen}>
            在访达中打开
          </button>
        </div>
      ) : null}
      {failure ? (
        <p className="field__error" role="alert">
          {failure}
        </p>
      ) : null}
    </div>
  );
}

/** The settings screen (desktop only). */
export default function SettingsScreen({ bridge = desktop }: { bridge?: Desktop }) {
  const { store } = useKb();
  const available = bridge.active && store.kind === "tauri";
  const [info, setInfo] = useState<SettingsInfo | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);

  useEffect(() => {
    if (!available) return;
    let alive = true;
    bridge.call<SettingsInfo>("settings_get").then(
      (loaded) => {
        if (alive) setInfo(loaded);
      },
      (error: unknown) => {
        if (alive) setFailure(errorText(error));
      },
    );
    return () => {
      alive = false;
    };
  }, [available, bridge]);

  const open = async (which: "logs" | "runs" | "bridge") => {
    setOpening(null);
    try {
      await bridge.call("settings_open_folder", { which });
    } catch (error) {
      setOpening(errorText(error));
    }
  };

  if (!available) {
    return (
      <div className="screen settings">
        <PageHeader eyebrow="设置" title="设置" />
        <EmptyState title="设置只在 Mac 上的底色里">网页版和演示用的是固定的设置。</EmptyState>
      </div>
    );
  }

  return (
    <div className="screen settings">
      <PageHeader
        eyebrow="设置"
        title="这台 Mac 上的底色"
        meta={info ? <span>版本 <span className="num">{info.version}</span></span> : null}
      />
      {failure ? (
        <p className="field__error" role="alert">
          读不到设置：{failure}
        </p>
      ) : null}
      {!info ? (
        failure ? null : <p className="muted">正在读取设置…</p>
      ) : (
        <div className="settings-grid">
          {info.configProblem ? (
            <p className="settings-banner" role="alert">
              {info.configProblem}
            </p>
          ) : null}
          <Section title="AI 引擎" aside="入库和落裁决用">
            <AiSettingsForm info={info} bridge={bridge} onSaved={setInfo} />
          </Section>
          <Section title="知识库" aside="一个 git 仓库">
            <KbSettings info={info} bridge={bridge} onSaved={setInfo} />
          </Section>
          <Section title="记一笔的快捷键" aside="在哪个 App 里都能用">
            <ShortcutSettings
              status={info.captureShortcut}
              bridge={bridge}
              onChanged={(next) => setInfo((current) => (current ? { ...current, captureShortcut: next } : current))}
            />
          </Section>
          {info.bridge ? (
            <Section title="iPhone" aside="只用 iCloud 云盘">
              <BridgeSettings status={info.bridge} bridge={bridge} onSaved={setInfo} onOpen={() => void open("bridge")} />
            </Section>
          ) : null}
          <Section title="开机时">
            <AutostartSwitch
              enabled={info.autostart}
              bridge={bridge}
              onChanged={(next) => setInfo((current) => (current ? { ...current, autostart: next } : current))}
            />
          </Section>
          <Section title="文件夹">
            <div className="settings-form__actions">
              <button type="button" className="btn" onClick={() => void open("logs")}>
                打开日志文件夹
              </button>
              <button type="button" className="btn" onClick={() => void open("runs")}>
                打开运行记录文件夹
              </button>
            </div>
            <p className="field__hint">日志不含知识库的内容；运行记录含有知识库内容，只留在这台 Mac 上。</p>
            {opening ? (
              <p className="field__error" role="alert">
                {opening}
              </p>
            ) : null}
          </Section>
          <p className="settings-version">
            底色 <span className="num">{info.version}</span>
          </p>
        </div>
      )}
    </div>
  );
}
