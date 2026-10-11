// Store layer: access to knowledge-base content (see types.ts).

export type { KbFileEntry, KbStore, KbStoreKind } from "./types";
export { KbFileExistsError } from "./types";
export { checkKbPath, assertKbPath, isInsideRoot } from "./kbPath";
export { createMemoryStore } from "./memoryStore";
export { canWrite, chooseStoreKind, createStore, storeLabel } from "./selectStore";
