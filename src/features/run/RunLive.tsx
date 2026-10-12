import { useEffect, useState } from "react";
import type { RunRecord, RunStep } from "../../app/runs";
import { useRun } from "../../app/RunContext";
import { ENGINE_LABEL, formatClock, phaseLabel, runDurationMs, TASK_LABEL, usageText } from "../../app/runView";
import { StepList } from "./StepList";

/** The live view of a run that is going on: where it stands, how long it runs, the steps. */
export function RunLive({ record, steps }: { record: RunRecord; steps: readonly RunStep[] }) {
  const run = useRun();
  const [now, setNow] = useState(() => Date.now());
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cancellable = record.phase === "starting" || record.phase === "running";

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!cancellable) setStopping(false);
  }, [cancellable]);

  const usage = usageText(record);
  const model = record.engineReport?.stream.model ?? record.model;

  const stop = async () => {
    setStopping(true);
    setError(null);
    try {
      await run.cancel();
    } catch (failure) {
      setStopping(false);
      setError(failure instanceof Error ? failure.message : String(failure));
    }
  };

  return (
    <section className="run-live" aria-label={`${TASK_LABEL[record.task]}进行中`}>
      <div className="run-live__head">
        <div className="run-live__meter">
          <div className="run-live__status">
            <span className="run-live__ink" aria-hidden="true" />
            <span className="run-live__phase">{stopping ? "正在停止…" : phaseLabel(record.phase, record.task)}</span>
          </div>
          <span className="run-live__clock num" aria-label="已经运行">
            {formatClock(runDurationMs(record, now))}
          </span>
        </div>
        <p className="run-live__engine">
          {ENGINE_LABEL[record.engine]} · {model ?? "默认模型"}
          {usage ? <span className="run-live__usage"> · {usage}</span> : null}
        </p>
        <div className="run-live__actions">
          <button type="button" className="btn btn--small" onClick={() => void stop()} disabled={!cancellable || stopping}>
            {stopping ? "正在停止…" : "取消"}
          </button>
          <span className="run-live__hint">收起这个面板不会停下；左边栏一直显示进度。</span>
        </div>
        {error ? (
          <p className="run-error" role="alert">
            {error}
          </p>
        ) : null}
      </div>
      <StepList steps={steps} follow />
    </section>
  );
}
