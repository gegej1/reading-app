import { cx } from "../ui/components";
import { useRun } from "./RunContext";
import { TASK_LABEL, waitingLabel } from "./runView";

/**
 * The run's place in the sidebar (desktop): 入库中 while a run goes on,
 * 待处理 while its result waits for a decision, 有结果 for a result not seen
 * yet; nothing otherwise (今日 leads to the 运行记录). A click opens the run panel.
 */
export function RunIndicator() {
  const run = useRun();
  if (!run.available || (!run.activeTask && !run.unseen)) return null;
  const waiting = run.status?.phase === "needsDecision";
  const asking = waiting && run.status?.run?.reason === "needsApproval";
  const tone = run.activeTask ? (waiting ? "attention" : "running") : "result";
  const label = run.activeTask ? (asking ? "待确认" : waiting ? "待处理" : `${TASK_LABEL[run.activeTask]}中`) : "有结果";
  const title = run.activeTask
    ? waiting && run.status?.run
      ? `${TASK_LABEL[run.activeTask]}${waitingLabel(run.status.run)}。点一下打开`
      : `正在${TASK_LABEL[run.activeTask]}，点一下看进度`
    : "有一次运行结束了，点一下看结果";
  return (
    <button
      type="button"
      className={cx("run-indicator", `is-${tone}`)}
      onClick={() => run.openPanel("current")}
      title={title}
      aria-label={title}
    >
      <span className="run-indicator__mark" aria-hidden="true" />
      <span className="run-indicator__label">{label}</span>
    </button>
  );
}
