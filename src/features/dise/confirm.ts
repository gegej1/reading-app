import { setConfirmed } from "../../domain";

/** Commit message for a confirmation change, e.g. `底色：确认 D05`. */
export function confirmMessage(id: string, confirmed: boolean): string {
  return `底色：${confirmed ? "确认" : "取消确认"} ${id}`;
}

/**
 * The edit behind the 盖章确认 / 取消确认 button. It carries the user's intent
 * (the state the button offered), not a flip: if the file is already in that
 * state, the edit changes nothing and nothing is committed.
 */
export function confirmEdit(id: string, desired: boolean) {
  return {
    edit: (current: string) => setConfirmed(current, id, desired),
    message: () => confirmMessage(id, desired),
  };
}
