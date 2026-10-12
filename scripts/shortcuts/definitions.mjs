// The three iOS shortcuts of the iPhone bridge, as plain data, and a tiny
// XML property-list writer. Pure: no file system, no clock, no randomness
// (action UUIDs are derived from fixed names), so the output is the same on
// every run and can be checked in (shortcuts/*.plist) and tested.
//
// The shortcuts only read and write the bridge folder 底色/ inside the
// Shortcuts app's iCloud folder:
//   记一笔  share sheet or typed/dictated text -> 底色/收件/记一笔/<time>.txt
//   看底色  a menu: 今日一条 (shown as text) and the exported pages (Quick Look)
//   答裁决  pick an open ruling, read it, answer -> 底色/收件/回答/Q-xxx-<time>.txt

import { createHash } from "node:crypto";

export const BRIDGE = "底色";
/** The Format Date pattern; the Mac reads this stamp back from the file name. */
export const STAMP_FORMAT = "yyyy-MM-dd'T'HH-mm-ss";
const OBJECT_REPLACEMENT = "\uFFFC";

/** A stable UUID for one action of one shortcut. */
export function stableUuid(seed) {
  const hex = createHash("sha256").update(seed).digest("hex").toUpperCase();
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-8${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
}

const output = (uuid, name) => ({ Type: "ActionOutput", OutputUUID: uuid, OutputName: name });
const SHORTCUT_INPUT = { Type: "ExtensionInput" };
const CURRENT_DATE = { Type: "CurrentDate" };

/** A whole parameter that is one variable. */
const variable = (ref) => ({ Value: ref, WFSerializationType: "WFTextTokenAttachment" });

/** Text with variables in it: strings and variable references, in order. */
export function tokenText(...parts) {
  let string = "";
  const attachmentsByRange = {};
  for (const part of parts) {
    if (typeof part === "string") {
      string += part;
    } else {
      // NSRange in UTF-16 code units, which is what JavaScript counts.
      attachmentsByRange[`{${string.length}, 1}`] = part;
      string += OBJECT_REPLACEMENT;
    }
  }
  return { Value: { string, attachmentsByRange }, WFSerializationType: "WFTextTokenString" };
}

const action = (id, parameters) => ({
  WFWorkflowActionIdentifier: `is.workflow.actions.${id}`,
  WFWorkflowActionParameters: parameters,
});

function shortcut({ color, glyph, actions, share = false }) {
  return {
    WFWorkflowMinimumClientVersion: 900,
    WFWorkflowMinimumClientVersionString: "900",
    WFWorkflowClientVersion: "1146.14",
    WFWorkflowIcon: { WFWorkflowIconStartColor: color, WFWorkflowIconGlyphNumber: glyph },
    WFWorkflowImportQuestions: [],
    WFWorkflowTypes: share ? ["ActionExtension"] : [],
    WFWorkflowInputContentItemClasses: share
      ? ["WFStringContentItem", "WFRichTextContentItem", "WFURLContentItem"]
      : [],
    WFWorkflowOutputContentItemClasses: [],
    WFWorkflowHasShortcutInputVariables: share,
    ...(share
      ? {
          // Run on its own (not from the share sheet): ask for the text.
          WFWorkflowNoInputBehavior: {
            Name: "WFWorkflowNoInputBehaviorAskForInput",
            Parameters: { ItemClass: "WFStringContentItem" },
          },
        }
      : {}),
    WFWorkflowActions: actions,
  };
}

/** Get File from the Shortcuts folder, without a picker. */
const getFile = (uuid, path) =>
  action("documentpicker.open", {
    UUID: uuid,
    WFGetFilePath: path,
    WFShowFilePicker: false,
    WFFileErrorIfNotFound: true,
  });

const textOf = (uuid, ref) => action("detect.text", { UUID: uuid, WFInput: variable(ref) });

const formatNow = (uuid) =>
  action("format.date", {
    UUID: uuid,
    WFDateFormatStyle: "Custom",
    WFDateFormat: STAMP_FORMAT,
    WFDate: tokenText(CURRENT_DATE),
  });

const saveInto = (uuid, ref, folder) =>
  action("documentpicker.save", {
    UUID: uuid,
    WFInput: variable(ref),
    WFAskWhereToSave: false,
    WFFileDestinationPath: `/${BRIDGE}/${folder}/`,
    WFSaveFileOverwrite: false,
  });

const show = (text) => action("showresult", { Text: text });

/** 记一笔: the shared or typed text, saved verbatim as one file. */
export function captureShortcut() {
  const id = (name) => stableUuid(`记一笔/${name}`);
  const text = id("text");
  const stamp = id("stamp");
  const named = id("named");
  return shortcut({
    color: 4271458815,
    glyph: 59446,
    share: true,
    actions: [
      textOf(text, SHORTCUT_INPUT),
      formatNow(stamp),
      action("setitemname", {
        UUID: named,
        WFInput: variable(output(text, "Text")),
        WFName: tokenText(output(stamp, "Formatted Date"), ".txt"),
        WFDontIncludeFileExtension: false,
      }),
      saveInto(id("saved"), output(named, "Renamed Item"), "收件/记一笔"),
      show("已记下"),
    ],
  });
}

/** The pages 看底色 offers, with the exported file each one opens. */
export const READING_MENU = [
  { title: "今日一条", file: "今日一条.txt", asText: true },
  { title: "整张底色", file: "底色.html" },
  { title: "待裁决", file: "待裁决.html" },
  { title: "待抽查", file: "待抽查.html" },
  { title: "判例", file: "判例.html" },
  { title: "最近入库", file: "最近入库.html" },
];

/** 看底色: a menu over the exported pages. */
export function readShortcut() {
  const group = stableUuid("看底色/menu");
  const actions = [
    action("choosefrommenu", {
      WFControlFlowMode: 0,
      GroupingIdentifier: group,
      WFMenuPrompt: "看底色",
      WFMenuItems: READING_MENU.map((item) => item.title),
    }),
  ];
  for (const item of READING_MENU) {
    const id = (name) => stableUuid(`看底色/${item.title}/${name}`);
    actions.push(
      action("choosefrommenu", {
        WFControlFlowMode: 1,
        GroupingIdentifier: group,
        WFMenuItemTitle: item.title,
      }),
      getFile(id("file"), `${BRIDGE}/阅读/${item.file}`),
    );
    if (item.asText) {
      actions.push(textOf(id("text"), output(id("file"), "File")), show(tokenText(output(id("text"), "Text"))));
    } else {
      actions.push(action("previewdocument", { WFInput: variable(output(id("file"), "File")) }));
    }
  }
  actions.push(
    action("choosefrommenu", {
      WFControlFlowMode: 2,
      GroupingIdentifier: group,
      UUID: stableUuid("看底色/menu-end"),
    }),
  );
  return shortcut({ color: 463140863, glyph: 59511, actions });
}

/** 答裁决: choose an open ruling, read it, answer it. */
export function answerShortcut() {
  const id = (name) => stableUuid(`答裁决/${name}`);
  const list = id("list");
  const listText = id("list-text");
  const lines = id("lines");
  const chosen = id("chosen");
  const matches = id("matches");
  const ruling = id("ruling");
  const question = id("question");
  const questionText = id("question-text");
  const answer = id("answer");
  const stamp = id("stamp");
  const named = id("named");
  const rulingRef = output(ruling, "Item from List");
  return shortcut({
    color: 2846468607,
    glyph: 59771,
    actions: [
      getFile(list, `${BRIDGE}/阅读/待裁决列表.txt`),
      textOf(listText, output(list, "File")),
      action("text.split", {
        UUID: lines,
        text: variable(output(listText, "Text")),
        WFTextSeparator: "New Lines",
      }),
      action("choosefromlist", {
        UUID: chosen,
        WFInput: variable(output(lines, "Split Text")),
        WFChooseFromListActionPrompt: "回答哪一个？",
        WFChooseFromListActionSelectMultiple: false,
      }),
      action("text.match", {
        UUID: matches,
        text: tokenText(output(chosen, "Chosen Item")),
        WFMatchTextPattern: "Q-[0-9]+",
        WFMatchTextCaseSensitive: true,
      }),
      action("getitemfromlist", {
        UUID: ruling,
        WFInput: variable(output(matches, "Matches")),
        WFItemSpecifier: "First Item",
      }),
      getFile(question, tokenText(`${BRIDGE}/阅读/待裁决/`, rulingRef, ".txt")),
      textOf(questionText, output(question, "File")),
      show(tokenText(output(questionText, "Text"))),
      action("ask", {
        UUID: answer,
        WFAskActionPrompt: tokenText(rulingRef, " 你怎么定？（可以按键盘上的麦克风说）"),
        WFInputType: "Text",
        WFAskActionDefaultAnswer: "",
      }),
      formatNow(stamp),
      action("setitemname", {
        UUID: named,
        WFInput: variable(output(answer, "Provided Input")),
        WFName: tokenText(rulingRef, "-", output(stamp, "Formatted Date"), ".txt"),
        WFDontIncludeFileExtension: false,
      }),
      saveInto(id("saved"), output(named, "Renamed Item"), "收件/回答"),
      show(tokenText("已交上 ", rulingRef, " 的回答。Mac 上的底色收到后，会放进裁决收件箱，等下次落裁决。")),
    ],
  });
}

/** Every shortcut by its name (the file name on the phone). */
export function allShortcuts() {
  return { 记一笔: captureShortcut(), 看底色: readShortcut(), 答裁决: answerShortcut() };
}

const escapeXml = (text) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function plistValue(value, depth) {
  const pad = "\t".repeat(depth);
  if (typeof value === "string") return `${pad}<string>${escapeXml(value)}</string>`;
  if (typeof value === "boolean") return `${pad}<${value}/>`;
  if (typeof value === "number") {
    if (!Number.isInteger(value)) throw new Error(`only whole numbers: ${value}`);
    return `${pad}<integer>${value}</integer>`;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return `${pad}<array/>`;
    return [`${pad}<array>`, ...value.map((item) => plistValue(item, depth + 1)), `${pad}</array>`].join("\n");
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value);
    if (entries.length === 0) return `${pad}<dict/>`;
    const lines = [`${pad}<dict>`];
    for (const [key, item] of entries) {
      lines.push(`${pad}\t<key>${escapeXml(key)}</key>`, plistValue(item, depth + 1));
    }
    lines.push(`${pad}</dict>`);
    return lines.join("\n");
  }
  throw new Error(`cannot write ${String(value)} into a property list`);
}

/** An XML property list. */
export function toPlist(value) {
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">',
    plistValue(value, 0),
    "</plist>",
    "",
  ].join("\n");
}
