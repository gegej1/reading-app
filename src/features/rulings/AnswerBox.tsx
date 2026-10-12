import { useMemo } from "react";
import { stripMarkdown, type Ruling } from "../../domain";
import { formatAnswerTime, useSaveAnswer } from "../../app/answers";
import { useKb } from "../../app/KbContext";
import { useRun } from "../../app/RunContext";
import { CapturePanel } from "../capture/CapturePanel";
import { answerDraftKey, createDraftStore, pageStorage } from "../capture/drafts";
import { Chip } from "../../ui/components";

export const READ_ONLY_ANSWER = "开发预览是只读的：请在桌面 App 里回答";

/**
 * The answer box of a pending ruling: the capture slip, kept as a draft per
 * question. An answer goes to inbox/裁决/ and waits for the next 落裁决;
 * a question may get more than one.
 */
export function AnswerBox({ ruling }: { ruling: Ruling }) {
  const { model, writable } = useKb();
  const { answerBlocked } = useRun();
  const save = useSaveAnswer();
  const drafts = useMemo(() => createDraftStore(pageStorage(), answerDraftKey(ruling.id)), [ruling.id]);
  const queued = model.answerIndex.rulings.get(ruling.id) ?? [];
  const blocked = writable ? answerBlocked : READ_ONLY_ANSWER;

  return (
    <section className="answer" aria-label={`回答 ${ruling.id}`}>
      <header className="answer__head">
        <h2 className="answer__title">你的回答</h2>
        {queued.length > 0 ? <Chip tone="jade">已回答，待落裁决</Chip> : null}
      </header>
      {ruling.question ? (
        <p className="answer__question">
          <span className="answer__label">要你定</span>
          {stripMarkdown(ruling.question)}
        </p>
      ) : null}
      {queued.length > 0 ? (
        <ol className="answer__queued" aria-label="已经交上的回答">
          {queued.map((answer) => (
            <li key={answer.path} className="answer__item">
              <blockquote className="answer__words">{answer.text}</blockquote>
              <p className="answer__meta">
                {formatAnswerTime(answer.time)} · 等下一次落裁决整理
              </p>
            </li>
          ))}
        </ol>
      ) : null}
      <div className="answer__slip">
        <CapturePanel
          key={ruling.id}
          variant="inline"
          drafts={drafts}
          blocked={blocked}
          save={async (text) => {
            await save({ kind: "ruling", id: ruling.id }, text);
          }}
          copy={{
            title: queued.length > 0 ? "再补一句" : "回答",
            placeholder: "用你自己的话说，这件事怎么定……",
            idle: "放进 inbox/裁决/，下次落裁决时整理成判例或底色",
            saved: "已交上，待落裁决",
            save: "交上回答",
            label: `回答 ${ruling.id}`,
          }}
        />
      </div>
    </section>
  );
}
