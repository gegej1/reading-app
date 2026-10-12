import { useState } from "react";
import { Link } from "react-router";
import type { GateIssue, RunTask } from "../../domain";
import { useKb } from "../../app/KbContext";
import { pageRoute } from "../../app/links";
import { useRun } from "../../app/RunContext";
import type { RunRecord, RunStep } from "../../app/runs";
import {
  ENGINE_LABEL,
  finalFigures,
  finalNote,
  finalReportPath,
  formatDuration,
  otherEngine,
  resultKind,
  runDurationMs,
  TASK_LABEL,
  usageText,
} from "../../app/runView";
import { cx, Hint } from "../../ui/components";
import { chineseNumber } from "../../ui/format";
import { ChangedFiles } from "./DiffView";
import { RunLive } from "./RunLive";
import { StepList } from "./StepList";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** The problems of a check, numbered 一、二、三 like the items of a document. */
export function IssueList({
  issues,
  tone,
  onOpen,
}: {
  issues: readonly GateIssue[];
  tone: "violation" | "warning";
  onOpen?: (path: string) => void;
}) {
  if (issues.length === 0) return null;
  return (
    <ol className={cx("issues", `issues--${tone}`)}>
      {issues.map((issue, index) => (
        <li key={`${issue.rule}-${issue.path ?? ""}-${index}`} className="issue">
          <span className="issue__no" aria-hidden="true">
            {chineseNumber(index + 1)}
          </span>
          <p className="issue__text">{issue.message}</p>
          {issue.path && onOpen ? (
            <button type="button" className="issue__open" onClick={() => onOpen(issue.path ?? "")}>
              看改动
            </button>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

/** Cost, duration and commit of a run, on one line. */
export function RunFacts({ record }: { record: RunRecord }) {
  const usage = usageText(record);
  const engineMs = record.engineReport?.durationMs ?? null;
  return (
    <p className="run-facts">
      <span>
        {ENGINE_LABEL[record.engine]} · {record.engineReport?.stream.model ?? record.model ?? "默认模型"}
      </span>
      {usage ? <span>花费 {usage}</span> : null}
      <span>用时 {formatDuration(runDurationMs(record))}</span>
      {engineMs !== null && engineMs > 0 ? <span>AI 工作 {formatDuration(engineMs)}</span> : null}
      {record.commit ? (
        <span>
          提交 <code>{record.commit.slice(0, 10)}</code>
        </span>
      ) : null}
    </p>
  );
}

/** Starting again: the same engine, or the other one. */
function Retry({ task, engine }: { task: RunTask; engine: RunRecord["engine"] }) {
  const run = useRun();
  const other = otherEngine(engine);
  return (
    <div className="run-actions">
      <button type="button" className="btn btn--seal-outline" onClick={() => run.requestStart(task, engine)}>
        再试一次
      </button>
      <button type="button" className="btn" onClick={() => run.requestStart(task, other)}>
        换用 {ENGINE_LABEL[other]} 再试
      </button>
    </div>
  );
}

function Success({ record }: { record: RunRecord }) {
  const run = useRun();
  const { model } = useKb();
  const kept = record.outcome === "kept";
  const final = record.gate?.final ?? null;
  const figures = finalFigures(final, record.task);
  const note = finalNote(final);
  const reportPath = record.task === "ingest" ? finalReportPath(final) : null;
  const report = reportPath ? model.reports.find((candidate) => candidate.path === reportPath) : undefined;
  const toAudit = report
    ? report.auditItems.filter((item) => !item.checked).length
    : Number(final?.["判例裁决"] ?? 0) || 0;
  const label = TASK_LABEL[record.task];

  return (
    <div className="run-done">
      <div className="run-done__stamp" aria-hidden="true">
        <span>{kept ? "保留" : record.task === "ingest" ? "已入库" : "已落定"}</span>
      </div>
      <h3 className="run-done__title">{kept ? `检查没通过，按你的要求提交了` : `${label}完成了`}</h3>
      <p className="run-done__summary">{record.gate?.summary ?? record.message}</p>
      {figures.length > 0 ? (
        <dl className="run-done__figures">
          {figures.map((figure) => (
            <div key={figure.label} className="figure">
              <dd className="figure__value num">{figure.value}</dd>
              <dt className="figure__label">{figure.label}</dt>
            </div>
          ))}
        </dl>
      ) : null}
      <RunFacts record={record} />
      {note ? <p className="run-done__note">AI 的备注：{note}</p> : null}
      {kept && record.keptDespite.length > 0 ? (
        <details className="run-details">
          <summary>当时没通过的 {record.keptDespite.length} 项</summary>
          <ul className="run-details__list">
            {record.keptDespite.map((message) => (
              <li key={message}>{message}</li>
            ))}
          </ul>
        </details>
      ) : null}
      {(record.gate?.warnings.length ?? 0) > 0 ? (
        <details className="run-details">
          <summary>{record.gate?.warnings.length} 条提醒（不影响提交）</summary>
          <IssueList issues={record.gate?.warnings ?? []} tone="warning" />
        </details>
      ) : null}
      <div className="run-actions">
        {reportPath ? (
          <Link to={pageRoute(reportPath)} className="btn btn--seal-outline" onClick={run.closePanel}>
            看入库报告
          </Link>
        ) : null}
        {record.task === "ingest" ? (
          toAudit > 0 ? (
            <Link to="/rulings?tab=audit" className="btn" onClick={run.closePanel}>
              去抽查 <span className="num">{toAudit}</span> 条
            </Link>
          ) : (
            <span className="run-actions__aside">这次没有要你抽查的裁决</span>
          )
        ) : (
          <Link to="/rulings" className="btn" onClick={run.closePanel}>
            去裁决台看看
          </Link>
        )}
      </div>
    </div>
  );
}

function Decision({ record }: { record: RunRecord }) {
  const run = useRun();
  const [busy, setBusy] = useState<"discard" | "keep" | "recheck" | null>(null);
  const [confirmKeep, setConfirmKeep] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const gate = record.gate;
  const violations = gate?.violations ?? [];
  const failure = record.engineReport && !record.engineReport.ok ? record.engineReport.failure : null;

  const act = async (what: "discard" | "keep" | "recheck") => {
    setBusy(what);
    setError(null);
    try {
      if (what === "discard") await run.discard(record.id);
      else if (what === "keep") await run.keep(record.id);
      else await run.recheck(record.id);
      setConfirmKeep(false);
    } catch (failed) {
      setError(errorText(failed));
    } finally {
      setBusy(null);
    }
  };

  const keepButton = (
    <button
      type="button"
      className="btn"
      onClick={() => setConfirmKeep(true)}
      disabled={busy !== null || gate === null || confirmKeep}
    >
      仍然提交
    </button>
  );

  return (
    <div className="run-decision">
      <header className="run-decision__head">
        <span className="run-decision__mark" aria-hidden="true">
          未过
        </span>
        <div>
          <h3 className="run-decision__title">没通过检查，改动原样留着，等你决定</h3>
          <p className="run-decision__message">{record.message}</p>
        </div>
      </header>

      {failure ? (
        <p className="run-decision__engine">
          AI 没有正常结束：{failure}。丢弃之后可以换用 {ENGINE_LABEL[otherEngine(record.engine)]} 再试。
        </p>
      ) : null}

      {violations.length > 0 ? (
        <section aria-label="没通过的检查">
          <h4 className="run-section__title">
            没通过的 <span className="num">{violations.length}</span> 项
          </h4>
          <IssueList issues={violations} tone="violation" onOpen={setFocus} />
        </section>
      ) : null}
      {(gate?.warnings.length ?? 0) > 0 ? (
        <details className="run-details">
          <summary>另有 {gate?.warnings.length} 条提醒（不影响提交）</summary>
          <IssueList issues={gate?.warnings ?? []} tone="warning" />
        </details>
      ) : null}

      <ChangedFiles runId={record.id} gate={gate} focus={focus} />

      <div className="run-decision__actions">
        <button
          type="button"
          className="btn btn--seal-outline"
          onClick={() => void act("discard")}
          disabled={busy !== null}
          title="把这次改过的文件还原，新建的删掉；改动先复制一份留在运行记录里"
        >
          {busy === "discard" ? "正在丢弃…" : "丢弃（可找回）"}
        </button>
        {gate === null ? <Hint text="还没有可用的检查结果，先重新检查一次">{keepButton}</Hint> : keepButton}
        <button type="button" className="btn" onClick={() => void act("recheck")} disabled={busy !== null}>
          {busy === "recheck" ? "正在检查…" : "重新检查"}
        </button>
      </div>
      <p className="run-decision__hint">在 Obsidian 或别的编辑器里改好文件后，点「重新检查」。</p>

      {confirmKeep ? (
        <div className="run-confirm" role="alertdialog" aria-label="仍然提交">
          <p className="run-confirm__text">
            仍然提交会<strong>跳过检查</strong>：上面 <span className="num">{violations.length}</span> 个问题会原样进入知识库，
            并记在运行记录里。确定要提交吗？
          </p>
          <div className="run-actions">
            <button type="button" className="btn btn--seal" onClick={() => void act("keep")} disabled={busy !== null}>
              {busy === "keep" ? "正在提交…" : "确定提交"}
            </button>
            <button type="button" className="btn btn--quiet" onClick={() => setConfirmKeep(false)} disabled={busy !== null}>
              再想想
            </button>
          </div>
        </div>
      ) : null}

      {error ? (
        <p className="run-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function Failure({ record }: { record: RunRecord }) {
  const cancelled = record.outcome === "cancelled";
  const exit = record.engineReport?.exit;
  const title = cancelled ? "已停止" : exit?.timedOut ? "超时了，已停下" : "没有完成";
  const failure = record.engineReport?.failure ?? null;
  return (
    <div className="run-failed">
      <h3 className="run-failed__title">
        {TASK_LABEL[record.task]}
        {title}
      </h3>
      <p className="run-failed__message">{record.message}</p>
      {failure && !(record.message ?? "").includes(failure) ? <p className="run-failed__detail">{failure}</p> : null}
      <p className="run-failed__calm">什么都没有提交，队列原样留着。</p>
      <RunFacts record={record} />
      <Retry task={record.task} engine={record.engine} />
    </div>
  );
}

function Plain({ record, title, hint }: { record: RunRecord; title: string; hint?: string }) {
  return (
    <div className="run-plain">
      <h3 className="run-plain__title">{title}</h3>
      <p className="run-plain__message">{record.message}</p>
      {hint ? <p className="run-plain__hint">{hint}</p> : null}
    </div>
  );
}

/** What became of a run: the live view while it goes on, then its result. */
export function RunResult({ record, steps }: { record: RunRecord; steps: readonly RunStep[] }) {
  const kind = resultKind(record);
  if (kind === "running") return <RunLive record={record} steps={steps} />;
  const label = TASK_LABEL[record.task];
  return (
    <div className="run-result">
      {kind === "success" ? <Success record={record} /> : null}
      {kind === "decision" ? <Decision record={record} /> : null}
      {kind === "failed" || kind === "cancelled" ? <Failure record={record} /> : null}
      {kind === "nothing" ? <Plain record={record} title="队列是空的" hint="没有启动 AI，也没有花钱。" /> : null}
      {kind === "refused" ? (
        <Plain record={record} title={`${label}没有开始`} hint="处理好上面说的问题，再点一次就行。" />
      ) : null}
      {kind === "discarded" ? (
        <Plain record={record} title="这次的改动已经丢弃" hint="改过的文件都复制了一份，留在运行记录里，需要时可以找回。" />
      ) : null}
      {steps.length > 0 ? (
        <details className="run-details run-details--steps">
          <summary>
            这次的每一步（<span className="num">{steps.length}</span>）
          </summary>
          <StepList steps={steps} />
        </details>
      ) : null}
    </div>
  );
}
