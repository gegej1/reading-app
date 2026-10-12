import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import type { GateFile, GateIssue, GateReport } from "../../domain";
import { checkedInput, watchCheckedInputs } from "../../app/gateHost";
import { useRun } from "../../app/RunContext";
import { cx } from "../../ui/components";
import { diffBlocks, diffFiles, splitMark, type DiffLine } from "../../ui/lineDiff";

const STATUS_TEXT: Record<GateFile["status"], string> = { added: "新建", modified: "改动", deleted: "删除" };
const OPAQUE_TEXT: Record<NonNullable<GateFile["opaque"]>, string> = {
  binary: "不是文本文件，没法逐字对照",
  symlink: "是符号链接，没法逐字对照",
  large: "文件太大，没有读出来",
};

/** CR at the end of a line is shown, so a change of line endings is visible. */
function LineText({ line }: { line: DiffLine }) {
  const cr = line.text.endsWith("\r");
  const shown = cr ? { ...line, text: line.text.slice(0, -1) } : line;
  const { before, marked, after } = splitMark(shown);
  return (
    <span className="diff__text">
      {before}
      {marked ? <mark className="diff__mark">{marked}</mark> : null}
      {after}
      {cr ? (
        <span className="diff__cr" title="这一行以 CR 结尾">
          ␍
        </span>
      ) : null}
    </span>
  );
}

function Row({ line }: { line: DiffLine }) {
  const sign = line.kind === "add" ? "+" : line.kind === "del" ? "−" : "";
  return (
    <div className={cx("diff__row", `is-${line.kind}`)}>
      <span className="diff__no num">{line.oldNo ?? ""}</span>
      <span className="diff__no num">{line.newNo ?? ""}</span>
      <span className="diff__sign" aria-label={line.kind === "add" ? "新增" : line.kind === "del" ? "删去" : undefined}>
        {sign}
      </span>
      <LineText line={line} />
    </div>
  );
}

/** A folded run of unchanged lines; a click shows them. */
function Gap({ lines }: { lines: DiffLine[] }) {
  const [open, setOpen] = useState(false);
  if (open) return lines.map((line, index) => <Row key={index} line={line} />);
  return (
    <button type="button" className="diff__gap" onClick={() => setOpen(true)}>
      ⋯ <span className="num">{lines.length}</span> 行没有变化
    </button>
  );
}

/**
 * The collation view (校勘) of one file: unified, line numbers of both sides,
 * removed lines in cinnabar with −, added lines in jade with +, the changed
 * characters inside a replaced line marked. Text wraps anywhere, so long
 * Chinese lines and paths stay readable.
 */
export function DiffView({ before, after }: { before: string | null; after: string | null }) {
  const diff = useMemo(() => diffFiles(before, after), [before, after]);
  const blocks = useMemo(() => diffBlocks(diff.lines), [diff]);
  if (diff.lines.length === 0) {
    return <p className="diff__empty">两边都是空的。</p>;
  }
  return (
    <div className="diff" role="table" aria-label="改动前后对照">
      {diff.tooLarge ? <p className="diff__note">两边差得太多，整个文件按"全部换掉"显示。</p> : null}
      {blocks.map((block, index) =>
        block.kind === "gap" ? (
          <Gap key={index} lines={block.lines} />
        ) : (
          block.lines.map((line, row) => <Row key={`${index}-${row}`} line={line} />)
        ),
      )}
      {diff.endNewlineChanged ? <p className="diff__note">文件末尾的换行也变了。</p> : null}
    </div>
  );
}

/** One changed file as the run panel lists it. */
export interface ChangedFile {
  path: string;
  status: GateFile["status"] | null;
  /** Known sides; undefined: still to be read. */
  before?: string | null;
  after?: string | null;
  opaque: GateFile["opaque"];
}

/** The changed files of a run: the gate's own input when this window checked it, else the report's list. */
export function useChangedFiles(
  runId: string,
  gate: GateReport | null,
): { files: ChangedFile[]; exact: boolean; version: number } {
  const input = useSyncExternalStore(watchCheckedInputs, () => checkedInput(runId));
  return useMemo(() => {
    if (input) {
      return {
        exact: true,
        version: input.attempt,
        files: input.changes.map((change) => ({
          path: change.path,
          status: change.status,
          before: change.before,
          after: change.after,
          opaque: change.opaque,
        })),
      };
    }
    const paths = [...new Set([...(gate?.commitPaths ?? []), ...(gate?.restoreQueuePaths ?? [])])];
    return { exact: false, version: 0, files: paths.map((path) => ({ path, status: null, opaque: null })) };
  }, [input, gate]);
}

/** One file of the list: opens its collation view. */
function FileItem({
  file,
  issues,
  open,
  onToggle,
}: {
  file: ChangedFile;
  issues: GateIssue[];
  open: boolean;
  onToggle(): void;
}) {
  const run = useRun();
  const [sides, setSides] = useState<{ before: string | null; after: string | null } | null>(
    file.before !== undefined || file.after !== undefined
      ? { before: file.before ?? null, after: file.after ?? null }
      : null,
  );
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || sides !== null || file.opaque) return;
    let alive = true;
    run.readSides(file.path).then(
      (read) => {
        if (alive) setSides(read);
      },
      (failure: unknown) => {
        if (alive) setError(failure instanceof Error ? failure.message : String(failure));
      },
    );
    return () => {
      alive = false;
    };
  }, [open, sides, file, run]);

  const status =
    file.status ?? (sides ? (sides.before === null ? "added" : sides.after === null ? "deleted" : "modified") : null);
  const counts = useMemo(() => (sides ? diffFiles(sides.before, sides.after) : null), [sides]);

  return (
    <li className={cx("changed", open && "is-open", issues.length > 0 && "has-issue")} id={`changed-${file.path}`}>
      <button type="button" className="changed__head" onClick={onToggle} aria-expanded={open}>
        <span className={cx("changed__status", status && `is-${status}`)}>{status ? STATUS_TEXT[status] : "改动"}</span>
        <span className="changed__path">{file.path}</span>
        {counts ? (
          <span className="changed__counts">
            <span className="changed__added num">+{counts.added}</span>
            <span className="changed__removed num">−{counts.removed}</span>
          </span>
        ) : null}
        {issues.length > 0 ? (
          <span className="changed__issue" title={issues.map((issue) => issue.message).join("\n")}>
            有问题
          </span>
        ) : null}
      </button>
      {open ? (
        <div className="changed__body">
          {file.opaque ? (
            <p className="diff__empty">{OPAQUE_TEXT[file.opaque]}</p>
          ) : error ? (
            <p className="diff__empty">读不出来：{error}</p>
          ) : sides ? (
            <DiffView before={sides.before} after={sides.after} />
          ) : (
            <p className="diff__empty">正在读取…</p>
          )}
        </div>
      ) : null}
    </li>
  );
}

/** The list of changed files with a collation view each; files with a violation first. */
export function ChangedFiles({
  runId,
  gate,
  focus,
}: {
  runId: string;
  gate: GateReport | null;
  /** A path to open (a violation was clicked). */
  focus: string | null;
}) {
  const { files, exact, version } = useChangedFiles(runId, gate);
  const issuesOf = (path: string) => (gate?.violations ?? []).filter((issue) => issue.path === path);
  const sorted = [...files].sort((a, b) => Number(issuesOf(b.path).length > 0) - Number(issuesOf(a.path).length > 0));
  const firstWithIssue = sorted.find((file) => issuesOf(file.path).length > 0)?.path ?? null;
  const [open, setOpen] = useState<Set<string>>(() => new Set(firstWithIssue ? [firstWithIssue] : []));

  useEffect(() => {
    if (!focus) return;
    setOpen((current) => new Set([...current, focus]));
    requestAnimationFrame(() => document.getElementById(`changed-${focus}`)?.scrollIntoView({ block: "nearest" }));
  }, [focus]);

  if (files.length === 0) return null;
  return (
    <section className="changed-files" aria-label="这次改了的文件">
      <h4 className="run-section__title">
        改了的文件 <span className="num">{files.length}</span>
        {exact ? null : <span className="run-section__aside">（和上次提交比）</span>}
      </h4>
      <ul className="changed-list">
        {sorted.map((file) => (
          <FileItem
            key={`${file.path}#${version}`}
            file={file}
            issues={issuesOf(file.path)}
            open={open.has(file.path)}
            onToggle={() =>
              setOpen((current) => {
                const next = new Set(current);
                if (next.has(file.path)) next.delete(file.path);
                else next.add(file.path);
                return next;
              })
            }
          />
        ))}
      </ul>
    </section>
  );
}
