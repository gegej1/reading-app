import { useEffect, useRef, useState, type ReactNode } from "react";
import { useKb } from "../../app/KbContext";
import { useRun, type StartRequest } from "../../app/RunContext";
import type { EngineKind, RunSettingsView } from "../../app/runs";
import { ENGINE_LABEL, formatBudget, queueText, TASK_LABEL } from "../../app/runView";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

const PURPOSE = {
  ingest: "把收件箱里的随手记逐字存进原料层，整理进知识层，对照底色和判例",
  landRulings: "把你对待裁决和抽查的回答落成判例、底色草稿或新的待裁决",
} as const;

/** What the engine and model of a run will be, as the backend chooses them (see run/settings.rs). */
export function chosenEngine(
  request: StartRequest,
  settings: RunSettingsView,
): { engine: EngineKind; model: string | null } {
  const engine = request.engine ?? settings.engine;
  // The configured model belongs to the configured engine.
  return { engine, model: engine === settings.engine ? settings.model : null };
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="sheet-row">
      <dt className="sheet-row__label">{label}</dt>
      <dd className="sheet-row__value">{children}</dd>
    </div>
  );
}

/**
 * The start sheet: what is about to run, on which engine and model, and
 * what it may cost. Nothing runs until 开始 is pressed.
 */
export function StartSheet({ request }: { request: StartRequest }) {
  const run = useRun();
  const { model: kb } = useKb();
  const [settings, setSettings] = useState<RunSettingsView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  const label = TASK_LABEL[request.task];
  const count = request.task === "ingest" ? kb.dashboard.inboxCaptures : kb.dashboard.answeredRulings;

  useEffect(() => {
    let alive = true;
    run.loadSettings().then(
      (loaded) => {
        if (alive) setSettings(loaded);
      },
      (failure: unknown) => {
        if (alive) setLoadError(errorText(failure));
      },
    );
    return () => {
      alive = false;
    };
    // The settings are read once per opening of the sheet.
  }, []);

  useEffect(() => {
    if (settings) confirm.current?.focus();
  }, [settings]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !starting) run.cancelStart();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [run, starting]);

  const chosen = settings ? chosenEngine(request, settings) : null;
  const program = chosen ? (chosen.engine === "claude" ? settings?.claudeProgram : settings?.codexProgram) : null;
  const claude = chosen?.engine === "claude";
  const minutes = settings ? Math.round(settings.runTimeoutMinutes) : null;

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      await run.confirmStart();
    } catch (failure) {
      setError(errorText(failure));
      setStarting(false);
    }
  };

  return (
    <div
      className="sheet-scrim"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !starting) run.cancelStart();
      }}
    >
      <section className="sheet" role="dialog" aria-modal="true" aria-label={`开始${label}`}>
        <header className="sheet__head">
          <p className="sheet__eyebrow">开始之前，先看一眼</p>
          <h2 className="sheet__title">{label}</h2>
          <p className="sheet__purpose">{PURPOSE[request.task]}</p>
        </header>

        {loadError ? (
          <p className="sheet__error" role="alert">
            读不到运行的设置：{loadError}
          </p>
        ) : null}

        <dl className="sheet__rows">
          <Row label="任务">{label}</Row>
          <Row label="队列">
            {request.task === "ingest" ? "inbox/ 里有 " : "inbox/裁决/ 里有 "}
            <span className="num">{count}</span>
            {queueText(request.task, count).replace(/^\d+/, "")}
          </Row>
          <Row label="引擎">{chosen ? `${ENGINE_LABEL[chosen.engine]}${claude ? " Code" : ""}` : "…"}</Row>
          <Row label="模型">
            {chosen ? (
              chosen.model ?? (
                <span className="sheet__muted">
                  默认模型 · 跟随你自己的 {ENGINE_LABEL[chosen.engine]} 设置
                </span>
              )
            ) : (
              "…"
            )}
          </Row>
          <Row label="每次上限">
            {settings && chosen ? (
              claude ? (
                <>
                  <span className="num">{formatBudget(settings.budgetUsd)}</span>（美元）
                </>
              ) : (
                <span className="sheet__muted">Codex 不支持花费上限</span>
              )
            ) : (
              "…"
            )}
          </Row>
          <Row label="时限">{minutes !== null ? `超过 ${minutes} 分钟就停下` : "…"}</Row>
        </dl>

        {settings && chosen ? (
          <p className="sheet__cost">
            {claude
              ? `按用量计费，这次最多花 ${formatBudget(settings.budgetUsd)}。`
              : "按用量计费，记在你自己的 Codex 账户上；App 没法给 Codex 设花费上限。"}
          </p>
        ) : null}

        <ul className="sheet__notes">
          <li>开始前先同步一次，把别处记下的内容拿到本机。</li>
          <li>AI 只改文件，不做 git 操作；App 检查通过才提交，没通过就等你决定。</li>
          <li>运行中照样可以记一笔；关掉运行面板也不会停下。</li>
        </ul>

        {settings && chosen && !program ? (
          <p className="sheet__warning" role="note">
            没在这台 Mac 上找到 {chosen.engine === "claude" ? "claude" : "codex"} 程序，运行多半会失败。可以在设置里看看。
          </p>
        ) : null}
        {settings?.problems.map((problem) => (
          <p key={problem} className="sheet__warning" role="note">
            {problem}
          </p>
        ))}
        {error ? (
          <p className="sheet__error" role="alert">
            没能开始：{error}
          </p>
        ) : null}

        <footer className="sheet__foot">
          <button type="button" className="btn btn--quiet" onClick={run.cancelStart} disabled={starting}>
            先不了
          </button>
          <button
            ref={confirm}
            type="button"
            className="btn btn--seal sheet__go"
            onClick={() => void start()}
            disabled={!settings || starting || count === 0}
          >
            {starting ? "正在开始…" : `开始${label}`}
          </button>
        </footer>
      </section>
    </div>
  );
}
