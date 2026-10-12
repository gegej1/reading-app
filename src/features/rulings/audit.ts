// The audit checklist (待你抽查) of ingest reports: ticking an item means
// "I agree with the AI's ruling" (认可). The edit carries the user's intent,
// like 盖章确认 on 底色: when the report already says so, nothing is written.

import { setChecked } from "../../domain";

/** `抽查：认可 I-12（2026-10-12-入库报告）`, or `抽查：取消认可 …` for un-ticking. */
export function auditMessage(itemId: string, reportName: string, checked: boolean): string {
  return `抽查：${checked ? "认可" : "取消认可"} ${itemId}（${reportName}）`;
}

/** The edit behind the checkbox of one item. */
export function auditEdit(itemId: string, reportName: string, desired: boolean) {
  return {
    edit: (current: string) => setChecked(current, itemId, desired),
    message: () => auditMessage(itemId, reportName, desired),
  };
}
