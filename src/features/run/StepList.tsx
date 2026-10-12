import { useLayoutEffect, useRef } from "react";
import type { RunStep } from "../../app/runs";
import { formatClock, stepTone, type StepTone } from "../../app/runView";
import { cx } from "../../ui/components";

const CHOP: Partial<Record<StepTone, string>> = {
  denied: "拒",
  text: "AI",
  note: "App",
  init: "起",
  result: "毕",
  error: "误",
};

/** The little square the verb is set in, like a chop on a ledger line. */
function Chop({ step, tone }: { step: RunStep; tone: StepTone }) {
  const text = step.kind === "tool" || step.kind === "denied" ? (step.verb ?? "工具") : (CHOP[tone] ?? "·");
  return <span className={cx("step__chop", `step__chop--${tone}`)}>{text}</span>;
}

/**
 * The steps of a run as a running ledger (流水簿): the time in the margin, a
 * verb chop (读 / 写 / 改 / 查找 …) and the repo-relative path, short
 * pieces of what the AI said, the app's own notes, refusals in cinnabar.
 * `follow` keeps the newest line in view while the reader is at the bottom.
 */
export function StepList({ steps, follow = false }: { steps: readonly RunStep[]; follow?: boolean }) {
  const list = useRef<HTMLOListElement>(null);
  const atBottom = useRef(true);

  useLayoutEffect(() => {
    const element = list.current;
    const scroller = element?.closest(".run-panel__body");
    if (!follow || !element || !(scroller instanceof HTMLElement)) return;
    if (atBottom.current) scroller.scrollTop = scroller.scrollHeight;
  }, [steps, follow]);

  useLayoutEffect(() => {
    const scroller = list.current?.closest(".run-panel__body");
    if (!follow || !(scroller instanceof HTMLElement)) return;
    const onScroll = () => {
      atBottom.current = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => scroller.removeEventListener("scroll", onScroll);
  }, [follow]);

  if (steps.length === 0) {
    return <p className="steps__empty">还没有步骤。</p>;
  }

  return (
    <ol className="steps" ref={list} role="log" aria-label="运行的每一步">
      {steps.map((step) => {
        const tone = stepTone(step);
        return (
          <li key={`${step.runId}-${step.seq}`} className={cx("step", `step--${tone}`)}>
            <span className="step__time num">{step.elapsedMs > 0 ? formatClock(step.elapsedMs) : ""}</span>
            <Chop step={step} tone={tone} />
            <span className="step__body">
              {step.target ? <span className="step__target">{step.target}</span> : null}
              {step.text ? <span className="step__text">{step.text}</span> : null}
              {step.kind === "denied" ? <span className="step__denied">被拒绝</span> : null}
            </span>
          </li>
        );
      })}
    </ol>
  );
}
