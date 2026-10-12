import { useEffect, useRef } from "react";
import { useRun } from "../../app/RunContext";
import { TASK_LABEL } from "../../app/runView";
import { cx, EmptyState } from "../../ui/components";
import { RunHistory } from "./RunHistory";
import { RunResult } from "./RunResult";
import { StartSheet } from "./StartSheet";
import "../../styles/run.css";

/**
 * The run panel: a scroll unrolled from the right edge of the desk. 这一次
 * shows the run going on (live) or its result; 运行记录 the kept runs.
 * Closing it never stops a run.
 */
function RunPanel() {
  const run = useRun();
  const panel = useRef<HTMLElement>(null);
  const record = run.status?.run ?? null;
  const steps = record ? run.steps.filter((step) => step.runId === record.id) : [];
  const title = run.panelView === "history" ? "运行记录" : record ? TASK_LABEL[record.task] : "AI 运行";

  useEffect(() => {
    panel.current?.focus();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // The start sheet, when open, takes Esc for itself.
      if (event.key === "Escape" && !run.pendingStart) run.closePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [run]);

  return (
    <div
      className="run-scrim"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) run.closePanel();
      }}
    >
      <aside className="run-panel" role="dialog" aria-modal="false" aria-label="AI 运行" tabIndex={-1} ref={panel}>
        <header className="run-panel__head">
          <div>
            <p className="run-panel__eyebrow">AI 运行</p>
            <h2 className="run-panel__title">{title}</h2>
          </div>
          <button type="button" className="run-panel__close" onClick={run.closePanel} aria-label="收起运行面板">
            收起
          </button>
        </header>
        <div className="tabs run-panel__tabs" role="tablist" aria-label="运行">
          <button
            type="button"
            role="tab"
            aria-selected={run.panelView === "current"}
            className={cx("tab", run.panelView === "current" && "is-active")}
            onClick={() => run.openPanel("current")}
          >
            这一次
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={run.panelView === "history"}
            className={cx("tab", run.panelView === "history" && "is-active")}
            onClick={() => run.openPanel("history")}
          >
            运行记录
          </button>
        </div>
        <div className="run-panel__body" role="tabpanel">
          {run.panelView === "history" ? (
            <RunHistory />
          ) : record ? (
            <RunResult key={record.id} record={record} steps={steps} />
          ) : (
            <EmptyState title="这次打开 App 之后还没有运行过">
              在今日点「入库 ▶」开始；以前的运行在「运行记录」里。
            </EmptyState>
          )}
        </div>
      </aside>
    </div>
  );
}

/** Everything of runs that loads on demand: the start sheet and the panel. */
export default function RunOverlay() {
  const run = useRun();
  return (
    <>
      {run.panelOpen ? <RunPanel /> : null}
      {run.pendingStart ? <StartSheet request={run.pendingStart} /> : null}
    </>
  );
}
