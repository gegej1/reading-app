import { useMemo, useState } from "react";
import { Link } from "react-router";
import { auditKey, type AuditItem, type Report } from "../../domain";
import { formatAnswerTime, useSaveAnswer } from "../../app/answers";
import { useKb } from "../../app/KbContext";
import { docRoute } from "../../app/links";
import { useRun } from "../../app/RunContext";
import { CapturePanel } from "../capture/CapturePanel";
import { createDraftStore, pageStorage, rejectionDraftKey } from "../capture/drafts";
import { Chip, cx, Hint } from "../../ui/components";
import { auditEdit } from "./audit";

const READ_ONLY_AUDIT = "开发预览是只读的：请在桌面 App 里抽查";
const NO_ID = "这一条没有编号，没法勾选";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** The reason box of one item: the capture slip, kept as a draft per item. */
function RejectionBox({
  report,
  itemId,
  shownAt,
  blocked,
  onClose,
}: {
  report: Report;
  itemId: string;
  shownAt: number;
  blocked: string | null;
  onClose(): void;
}) {
  const save = useSaveAnswer();
  const drafts = useMemo(() => createDraftStore(pageStorage(), rejectionDraftKey(report.name, itemId)), [report.name, itemId]);
  return (
    <div className="audit-row__reject">
      <CapturePanel
        variant="inline"
        drafts={drafts}
        shownAt={shownAt}
        blocked={blocked}
        hide={onClose}
        save={async (text) => {
          await save({ kind: "audit", report: report.name, itemId }, text);
        }}
        copy={{
          title: "不认可的理由",
          placeholder: "这条哪里判错了？用你的话说……",
          idle: "放进 inbox/裁决/，落裁决时会新建一个待裁决",
          saved: "已记下，待落裁决",
          save: "交上理由",
          label: `不认可 ${itemId} 的理由`,
        }}
      />
    </div>
  );
}

/** One item of the checklist: 认可 (tick), 不认可 (a reason for 落裁决). */
export function AuditItemRow({ report, item }: { report: Report; item: AuditItem }) {
  const { model, writable, editing, editFile, notify } = useKb();
  const { editBlocked, answerBlocked } = useRun();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rejectingAt, setRejectingAt] = useState<number | null>(null);
  const itemId = item.itemId;
  const precedentPath = item.precedent ? model.links.index.resolve(item.precedent) : null;
  const precedent = precedentPath ? model.byPath.get(precedentPath) : undefined;
  const rejections = itemId ? (model.answerIndex.audits.get(auditKey(report.name, itemId)) ?? []) : [];
  const rejected = rejections.length > 0;

  const tickBlocked = !writable ? READ_ONLY_AUDIT : (editBlocked ?? (itemId ? null : NO_ID));
  const rejectBlocked = !writable ? READ_ONLY_AUDIT : answerBlocked;

  const toggle = async () => {
    if (!itemId || tickBlocked !== null) return;
    const desired = !item.checked;
    setPending(true);
    setError(null);
    try {
      const { edit, message } = auditEdit(itemId, report.name, desired);
      const result = await editFile(report.path, edit, message);
      notify(
        result.changed ? `已提交：${result.message}` : `${itemId} 本来就${desired ? "已经认可" : "没有勾选"}，没有改动`,
      );
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setPending(false);
    }
  };

  const box = (
    <button
      type="button"
      role="checkbox"
      aria-checked={item.checked}
      aria-label={`${item.checked ? "已认可" : "认可"} ${itemId ?? item.summary}`}
      className={cx("audit-row__box", item.checked && "is-checked", pending && "is-pending")}
      onClick={() => void toggle()}
      disabled={tickBlocked !== null || editing || pending}
    >
      {item.checked ? "✓" : ""}
    </button>
  );

  return (
    <li className={cx("audit-row", item.checked && "is-checked", rejected && "is-rejected")}>
      {tickBlocked !== null ? <Hint text={tickBlocked}>{box}</Hint> : box}
      <div className="audit-row__main">
        <p className="audit-row__summary">
          {itemId ? <span className="audit-row__id">{itemId}</span> : null}
          {item.summary}
        </p>
        {item.tension ? (
          <p className="audit-row__tension">
            <span className="audit-row__arrow" aria-label="对照">
              ↔
            </span>{" "}
            {item.tension}
          </p>
        ) : null}
        {item.ruling ? (
          <p className="audit-row__ruling">
            <span className="audit-row__label">AI 裁决</span>
            {item.ruling}
          </p>
        ) : null}
        {item.precedent ? (
          <p className="audit-row__basis">
            依据{" "}
            {precedent ? (
              <Link to={docRoute(precedent)} className="text-link">
                {precedent.name}
              </Link>
            ) : (
              <span className="wikilink--broken">{item.precedent}</span>
            )}
          </p>
        ) : null}
        {rejections.map((rejection) => (
          <blockquote key={rejection.path} className="audit-row__reason">
            {rejection.text}
            <span className="audit-row__reason-time">{formatAnswerTime(rejection.time)}</span>
          </blockquote>
        ))}
        {error ? (
          <p className="audit-row__error" role="alert">
            {error}
          </p>
        ) : null}
        {rejectingAt !== null && itemId ? (
          <RejectionBox
            report={report}
            itemId={itemId}
            shownAt={rejectingAt}
            blocked={rejectBlocked}
            onClose={() => setRejectingAt(null)}
          />
        ) : null}
      </div>
      <div className="audit-row__side">
        {item.checked ? <span className="audit-row__state">已认可</span> : null}
        {rejected ? <Chip tone="seal">已驳回，待落裁决</Chip> : null}
        {!item.checked && itemId && rejectingAt === null ? (
          <button
            type="button"
            className="btn btn--quiet btn--small audit-row__reject-button"
            onClick={() => setRejectingAt(Date.now())}
          >
            {rejected ? "补充理由" : "不认可"}
          </button>
        ) : null}
      </div>
    </li>
  );
}

/** The 待你抽查 checklist of one report. */
export function AuditChecklist({ report }: { report: Report }) {
  return (
    <ul className="audit-list">
      {report.auditItems.map((item) => (
        <AuditItemRow key={`${item.line}-${item.text}`} report={report} item={item} />
      ))}
    </ul>
  );
}
