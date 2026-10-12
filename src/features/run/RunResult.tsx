import { useEffect, useState } from "react";
import { Link } from "react-router";
import type { GateApproval, GateIssue, RunTask } from "../../domain";
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
import { ChangedFiles, DiffView, useChangedFiles } from "./DiffView";
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

/** The changes that wait for the user, numbered, each with what it changed. */
export function ApprovalList({ items, runId, withDiff }: { items: readonly GateApproval[]; runId: string; withDiff: boolean }) {
  if (items.length === 0) return null;
  return (
    <ol className="approvals">
      {items.map((item, index) => (
        <li key={`${item.rule}-${item.path}-${item.entry ?? ""}-${index}`} className="approval">
          <div className="approval__head">
            <span className="approval__no" aria-hidden="true">
              {chineseNumber(index + 1)}
            </span>
            <div className="approval__what">
              <p className="approval__text">{item.message}</p>
              <p className="approval__where">
                <code>{item.path}</code>
                {item.entry ? <span className="approval__entry">{item.entry}</span> : null}
              </p>
            </div>
          </div>
          {withDiff ? <ApprovalDiff item={item} runId={runId} /> : null}
        </li>
      ))}
    </ol>
  );
}

/** Before and after of one change waiting for approval: the part the gate named, else the whole file. */
function ApprovalDiff({ item, runId }: { item: GateApproval; runId: string }) {
  const run = useRun();
  const { files } = useChangedFiles(runId, null);
  const known = files.find((file) => file.path === item.path && (file.before !== undefined || file.after !== undefined));
  const own = item.before !== null || item.after !== null;
  const [read, setRead] = useState<{ before: string | null; after: string | null } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (own || known || read) return;
    let alive = true;
    run.readSides(item.path).then(
      (sides) => {
        if (alive) setRead(sides);
      },
      (failure: unknown) => {
        if (alive) setError(errorText(failure));
      },
    );
    return () => {
      alive = false;
    };
  }, [own, known, read, run, item.path]);
  const sides = own
    ? { before: item.before, after: item.after }
    : known
      ? { before: known.before ?? null, after: known.after ?? null }
      : read;
  return (
    <div className="approval__diff">
      {error ? (
        <p className="diff__empty">读不出来：{error}</p>
      ) : sides ? (
        <DiffView before={sides.before} after={sides.after} />
      ) : (
        <p className="diff__empty">正在读取…</p>
      )}
    </div>
  );
}

/** Cost, duration and commit of a run, on one line. */
export function RunFacts({ record }: { record: RunRecord }) {
  const usage = usageText(record);
  const engineMs = record.engineReport?.durationMs ?? null;
  const paid = record.engineReport?.stream.costUsd !== null && record.engineReport?.stream.costUsd !== undefined;
  return (
    <p className="run-facts">
      <span>
        {ENGINE_LABEL[record.engine]} · {record.engineReport?.stream.model ?? record.model ?? "默认模型"}
      </span>
      {usage ? <span>{paid ? "花费" : "用量"} {usage}</span> : null}
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
  const approved = record.decision === "approved";
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
        <span>{kept ? "保留" : approved ? "已确认" : record.task === "ingest" ? "已入库" : "已落定"}</span>
      </div>
      <h3 className="run-done__title">
        {kept ? `检查没通过，按你的要求提交了` : approved ? `已确认提交，${label}完成了` : `${label}完成了`}
      </h3>
      <p className="run-done__summary">{approved ? record.message : (record.gate?.summary ?? record.message)}</p>
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
      {record.approved.length > 0 ? (
        <details className="run-details">
          <summary>你确认过的 {record.approved.length} 处改动</summary>
          <ul className="run-details__list">
            {record.approved.map((message) => (
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
      {(gate?.approvals.length ?? 0) > 0 ? (
        <section aria-label="另外需要你确认的改动">
          <h4 className="run-section__title">
            另外需要你确认的 <span className="num">{gate?.approvals.length}</span> 项
          </h4>
          <ApprovalList items={gate?.approvals ?? []} runId={record.id} withDiff={false} />
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

/** The two buttons and the busy state shared by the calmer waiting screens. */
function useAct(record: RunRecord) {
  const run = useRun();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const act = async (what: string, action: (runId: string) => Promise<void>) => {
    setBusy(what);
    setError(null);
    try {
      await action(record.id);
    } catch (failed) {
      setError(errorText(failed));
    } finally {
      setBusy(null);
    }
  };
  return { run, busy, error, act };
}

/** Changes to the user's own layer (底色, systems/): nothing is wrong, but they decide. */
function Approval({ record }: { record: RunRecord }) {
  const { run, busy, error, act } = useAct(record);
  const gate = record.gate;
  const items = gate?.approvals ?? [];
  return (
    <div className="run-approval">
      <header className="run-approval__head">
        <span className="run-approval__mark" aria-hidden="true">
          待你
        </span>
        <div>
          <h3 className="run-approval__title">需要你确认</h3>
          <p className="run-approval__lead">
            这次有 <span className="num">{items.length}</span> 处改动动到了你自己的底色或体系。其余都通过了检查，改动原样留着，等你看过再决定。
          </p>
        </div>
      </header>

      <section aria-label="需要你确认的改动">
        <ApprovalList items={items} runId={record.id} withDiff />
      </section>

      {(gate?.warnings.length ?? 0) > 0 ? (
        <details className="run-details">
          <summary>另有 {gate?.warnings.length} 条提醒（不影响提交）</summary>
          <IssueList issues={gate?.warnings ?? []} tone="warning" />
        </details>
      ) : null}
      <details className="run-details">
        <summary>这次改了的全部文件</summary>
        <ChangedFiles runId={record.id} gate={gate} focus={null} />
      </details>

      <div className="run-approval__actions">
        <button type="button" className="btn btn--seal" onClick={() => void act("approve", run.approve)} disabled={busy !== null}>
          {busy === "approve" ? "正在提交…" : "确认并提交"}
        </button>
        <button
          type="button"
          className="btn"
          onClick={() => void act("discard", run.discard)}
          disabled={busy !== null}
          title="把这次改过的文件还原，新建的删掉；改动先复制一份留在运行记录里"
        >
          {busy === "discard" ? "正在丢弃…" : "丢弃（可找回）"}
        </button>
      </div>
      <p className="run-approval__hint">确认后按平常的方式提交并同步；丢弃会还原这次的全部改动，副本留在运行记录里。</p>
      {error ? (
        <p className="run-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** The check passed (or the user agreed), but git did not take the commit. */
function CommitFailed({ record }: { record: RunRecord }) {
  const { run, busy, error, act } = useAct(record);
  return (
    <div className="run-commit-failed">
      <h3 className="run-commit-failed__title">检查通过了，但没能提交</h3>
      <p className="run-commit-failed__message">{record.message}</p>
      <p className="run-commit-failed__calm">
        改动都还在，一个字也没丢。常见的原因是 git 钩子拒绝了提交，或者知识库文件夹的权限、磁盘出了问题；处理好以后点「重试提交」。
      </p>
      <RunFacts record={record} />
      <details className="run-details">
        <summary>这次改了的文件</summary>
        <ChangedFiles runId={record.id} gate={record.gate} focus={null} />
      </details>
      <div className="run-approval__actions">
        <button type="button" className="btn btn--seal" onClick={() => void act("retry", run.retryCommit)} disabled={busy !== null}>
          {busy === "retry" ? "正在提交…" : "重试提交"}
        </button>
        <button type="button" className="btn" onClick={() => void act("discard", run.discard)} disabled={busy !== null}>
          {busy === "discard" ? "正在丢弃…" : "丢弃（可找回）"}
        </button>
      </div>
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
      <p className="run-failed__calm">
        {(record.message ?? "").includes("队列原样留着") ? "什么都没有提交。" : "什么都没有提交，队列原样留着。"}
      </p>
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
      {kind === "approval" ? <Approval record={record} /> : null}
      {kind === "commitFailed" ? <CommitFailed record={record} /> : null}
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
