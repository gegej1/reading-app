import { useEffect, useState } from "react";
import { useRun } from "../../app/RunContext";
import type { RunRecord } from "../../app/runs";
import {
  ENGINE_LABEL,
  formatDuration,
  formatRunTime,
  outcomeLabel,
  runDurationMs,
  TASK_LABEL,
  usageShort,
  type RunDetail,
} from "../../app/runView";
import { cx, EmptyState } from "../../ui/components";
import { IssueList, RunFacts } from "./RunResult";
import { StepList } from "./StepList";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

function OutcomeChip({ record }: { record: RunRecord }) {
  const outcome = outcomeLabel(record);
  return <span className={cx("outcome", `outcome--${outcome.tone}`)}>{outcome.text}</span>;
}

/** One kept run: what happened, its check and its steps. */
function Detail({ id, onBack }: { id: string; onBack(): void }) {
  const run = useRun();
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    run.loadRun(id).then(
      (loaded) => {
        if (alive) setDetail(loaded);
      },
      (failure: unknown) => {
        if (alive) setError(errorText(failure));
      },
    );
    return () => {
      alive = false;
    };
    // Loaded once per run shown.
  }, [id]);

  const record = detail?.record;
  return (
    <div className="history-detail">
      <button type="button" className="history-detail__back" onClick={onBack}>
        ← 运行记录
      </button>
      {error ? (
        <p className="run-error" role="alert">
          {error}
        </p>
      ) : null}
      {!record ? (
        error ? null : <p className="steps__empty">正在读取…</p>
      ) : (
        <>
          <header className="history-detail__head">
            <h3 className="history-detail__title">
              {TASK_LABEL[record.task]}
              <span className="history-detail__time">{formatRunTime(record.startedAt)}</span>
            </h3>
            <OutcomeChip record={record} />
          </header>
          {record.message ? <p className="history-detail__message">{record.message}</p> : null}
          <RunFacts record={record} />
          <p className="history-detail__trigger">
            {record.trigger === "ui" ? "在 App 里发起" : "从命令行发起"} · 队列 <span className="num">{record.queue.length}</span> 个文件
          </p>
          {record.gate ? (
            <section className="history-detail__gate" aria-label="检查结果">
              <h4 className="run-section__title">检查结果</h4>
              <p className="history-detail__summary">{record.gate.summary}</p>
              <IssueList issues={record.gate.violations} tone="violation" />
              {record.gate.warnings.length > 0 ? (
                <details className="run-details">
                  <summary>{record.gate.warnings.length} 条提醒</summary>
                  <IssueList issues={record.gate.warnings} tone="warning" />
                </details>
              ) : null}
            </section>
          ) : null}
          {record.keptDespite.length > 0 ? (
            <section aria-label="保留时没通过的检查">
              <h4 className="run-section__title">保留时没通过的检查</h4>
              <ul className="run-details__list">
                {record.keptDespite.map((message) => (
                  <li key={message}>{message}</li>
                ))}
              </ul>
            </section>
          ) : null}
          {record.approved.length > 0 ? (
            <section aria-label="你确认过的改动">
              <h4 className="run-section__title">你确认过的改动</h4>
              <ul className="run-details__list">
                {record.approved.map((message) => (
                  <li key={message}>{message}</li>
                ))}
              </ul>
            </section>
          ) : null}
          {record.discardedTo ? (
            <p className="history-detail__copies">
              丢弃前的副本：<code>{record.discardedTo}</code>
            </p>
          ) : null}
          <section aria-label="每一步">
            <h4 className="run-section__title">
              每一步 <span className="num">{detail.steps.length}</span>
            </h4>
            <StepList steps={detail.steps} />
          </section>
        </>
      )}
    </div>
  );
}

/** The 运行记录: every kept run, newest first; opening one shows its steps and check. */
export function RunHistory() {
  const run = useRun();
  const [records, setRecords] = useState<RunRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    run.loadHistory().then(
      (list) => {
        if (alive) setRecords(list);
      },
      (failure: unknown) => {
        if (alive) setError(errorText(failure));
      },
    );
    return () => {
      alive = false;
    };
    // Loaded again whenever a run moves on.
  }, [run.status?.phase, run.status?.run?.id]);

  if (open) return <Detail id={open} onBack={() => setOpen(null)} />;
  if (error) {
    return (
      <p className="run-error" role="alert">
        读不到运行记录：{error}
      </p>
    );
  }
  if (!records) return <p className="steps__empty">正在读取…</p>;
  if (records.length === 0) {
    return <EmptyState title="还没有运行记录">每次入库或落裁决都会留一份记录，最新的 20 次留着。</EmptyState>;
  }
  return (
    <table className="history">
      <thead>
        <tr>
          <th scope="col">时间</th>
          <th scope="col">任务</th>
          <th scope="col">引擎 · 模型</th>
          <th scope="col">结果</th>
          <th scope="col">花费</th>
          <th scope="col">用时</th>
        </tr>
      </thead>
      <tbody>
        {records.map((record) => (
          <tr key={record.id} className="history__row" onClick={() => setOpen(record.id)}>
            <td className="history__time">
              <button type="button" className="history__open" onClick={() => setOpen(record.id)}>
                {formatRunTime(record.startedAt)}
              </button>
            </td>
            <td>{TASK_LABEL[record.task]}</td>
            <td className="history__engine">
              {ENGINE_LABEL[record.engine]}
              <span className="history__model">{record.model ?? "默认"}</span>
            </td>
            <td>
              <OutcomeChip record={record} />
            </td>
            <td className="history__usage">{usageShort(record) ?? "—"}</td>
            <td className="history__duration">{record.endedAt ? formatDuration(runDurationMs(record)) : "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
