// Domain layer: pure types and logic of the app.
// Nothing in here may import React or Tauri or perform I/O of its own, so
// everything stays unit-testable in plain Node. loadKb reads through an
// injected store.

export * from "./version";
export * from "./frontmatter";
export * from "./markdown";
export * from "./wikilinks";
export * from "./dise";
export * from "./precedents";
export * from "./rulings";
export * from "./reports";
export * from "./search";
export * from "./kb";
export * from "./pages";
export * from "./capture";
export * from "./gate";
