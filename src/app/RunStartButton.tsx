import type { RunTask } from "../domain";
import { cx, Hint } from "../ui/components";
import { useRun } from "./RunContext";
import { TASK_LABEL } from "./runView";

const EMPTY: Record<RunTask, string> = {
  ingest: "收件箱是空的",
  landRulings: "还没有回答（inbox/裁决/ 是空的）",
};

const UNIT: Record<RunTask, string> = { ingest: "条", landRulings: "份" };

/**
 * The button that starts 入库 or 落裁决: it opens the start sheet (nothing
 * runs before the user confirms there). It says why it cannot be used: on
 * the web, with an empty queue, or while a run holds the knowledge base (then
 * it opens the run panel instead).
 */
export function RunStartButton({ task, count, className }: { task: RunTask; count: number; className?: string }) {
  const run = useRun();
  const label = TASK_LABEL[task];

  if (!run.available) {
    const caption = "只能在 Mac 上运行";
    return (
      <span className={cx("run-start", className)}>
        <Hint text={caption}>
          <button type="button" className="btn btn--seal" disabled>
            {label} ▶
          </button>
        </Hint>
        <span className="run-start__caption">{caption}</span>
      </span>
    );
  }

  if (run.activeTask) {
    const busy = TASK_LABEL[run.activeTask];
    const waiting = run.status?.phase === "needsDecision";
    return (
      <span className={cx("run-start", className)}>
        <button type="button" className="btn btn--seal-outline run-start__live" onClick={() => run.openPanel("current")}>
          <span className="run-start__pulse" aria-hidden="true" />
          {waiting ? `${busy}的结果等你处理` : `正在${busy}…`}
        </button>
        <span className="run-start__caption">点开看进度</span>
      </span>
    );
  }

  if (count === 0) {
    return (
      <span className={cx("run-start", className)}>
        <Hint text={EMPTY[task]}>
          <button type="button" className="btn btn--seal" disabled>
            {label} ▶
          </button>
        </Hint>
        <span className="run-start__caption">{EMPTY[task]}</span>
      </span>
    );
  }

  return (
    <span className={cx("run-start", className)}>
      <button type="button" className="btn btn--seal" onClick={() => run.requestStart(task)}>
        {label} ▶
        <span className="run-start__count">
          <span className="num">{count}</span> {UNIT[task]}
        </span>
      </button>
      <span className="run-start__caption">开始前会先让你确认</span>
    </span>
  );
}
