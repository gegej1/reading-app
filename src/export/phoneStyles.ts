// The stylesheet of the exported phone pages: the app's study-room look
// (rice paper, warm ink, cinnabar seal, ochre drafts; see src/styles/tokens.css),
// phone width first, light and dark through prefers-color-scheme. Inlined
// into every page: the pages have no other files and no scripts.

export const PHONE_CSS = `
:root {
  color-scheme: light dark;
  --font-ui: -apple-system, BlinkMacSystemFont, "Helvetica Neue", "PingFang SC", "Hiragino Sans GB", sans-serif;
  --font-serif: "Songti SC", "STSong", "Noto Serif CJK SC", "Source Han Serif SC", serif;
  --font-num: "Iowan Old Style", "Palatino", Georgia, "Songti SC", serif;
  --paper: #f3ede1;
  --paper-raised: #fbf8f1;
  --paper-sunk: #e9e1d0;
  --paper-edge: #ddd2bd;
  --ink: #27221c;
  --ink-soft: #564d42;
  --ink-faint: #877c6c;
  --rule: rgba(39, 34, 28, 0.12);
  --seal: #b13a27;
  --seal-ink: #fbf3e6;
  --seal-wash: rgba(177, 58, 39, 0.08);
  --seal-line: rgba(177, 58, 39, 0.3);
  --ochre: #94651f;
  --ochre-wash: rgba(148, 101, 31, 0.11);
  --azurite: #2c5672;
  --shadow: 0 1px 0 rgba(255, 255, 255, 0.7) inset, 0 1px 2px rgba(39, 34, 28, 0.05), 0 10px 24px -16px rgba(39, 34, 28, 0.3);
}
@media (prefers-color-scheme: dark) {
  :root {
    --paper: #1b1915;
    --paper-raised: #24211c;
    --paper-sunk: #151310;
    --paper-edge: #35302a;
    --ink: #ebe3d3;
    --ink-soft: #c0b5a3;
    --ink-faint: #8f8574;
    --rule: rgba(235, 227, 211, 0.1);
    --seal: #d65f48;
    --seal-ink: #fff4ea;
    --seal-wash: rgba(214, 95, 72, 0.11);
    --seal-line: rgba(214, 95, 72, 0.34);
    --ochre: #d2a25b;
    --ochre-wash: rgba(210, 162, 91, 0.12);
    --azurite: #93bcd6;
    --shadow: 0 1px 0 rgba(255, 255, 255, 0.025) inset, 0 12px 28px -16px rgba(0, 0, 0, 0.7);
  }
}
*, *::before, *::after { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  font: 16px/1.75 var(--font-ui);
  color: var(--ink);
  background: var(--paper);
  -webkit-font-smoothing: antialiased;
  font-variant-numeric: lining-nums;
  overflow-wrap: anywhere;
}
a { color: var(--azurite); text-decoration-thickness: 1px; text-underline-offset: 3px; }
h1, h2, h3, h4, p { margin: 0; }
.num { font-family: var(--font-num); letter-spacing: 0.02em; }
.wrap {
  max-width: 680px;
  margin: 0 auto;
  padding: 0 max(16px, env(safe-area-inset-left)) 40px max(16px, env(safe-area-inset-right));
}

/* The masthead: a seal and the pages. */
.mast {
  padding: max(14px, env(safe-area-inset-top)) 0 10px;
  border-bottom: 1px solid var(--rule);
  margin-bottom: 22px;
}
.mast__row { display: flex; align-items: center; gap: 12px; }
.seal {
  display: inline-grid;
  place-items: center;
  width: 38px;
  height: 38px;
  flex: none;
  font: 700 14px/1.05 var(--font-serif);
  color: var(--seal-ink);
  background: var(--seal);
  border-radius: 4px;
  box-shadow: inset 0 0 0 2px var(--seal), inset 0 0 0 3px rgba(251, 243, 230, 0.75);
  text-decoration: none;
  writing-mode: vertical-rl;
  letter-spacing: 0.05em;
}
.mast__title { font: 600 15px/1.3 var(--font-serif); letter-spacing: 0.12em; color: var(--ink-soft); }
.mast__nav {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 12px;
}
.mast__nav a {
  flex: none;
  padding: 3px 11px;
  font-size: 14px;
  color: var(--ink-soft);
  text-decoration: none;
  border: 1px solid var(--paper-edge);
  border-radius: 999px;
  background: var(--paper-raised);
}
.mast__nav a[aria-current="page"] { color: var(--seal-ink); background: var(--seal); border-color: var(--seal); }

/* Page heads. */
.eyebrow { font-size: 12.5px; letter-spacing: 0.2em; color: var(--ink-faint); }
.title { margin-top: 2px; font: 700 26px/1.35 var(--font-serif); letter-spacing: 0.06em; }
.meta { margin-top: 6px; font-size: 13.5px; color: var(--ink-faint); }
.lede { margin: 14px 0 0; font-size: 15px; color: var(--ink-soft); }
.head { margin-bottom: 22px; }

/* Cards (entries, precedents, rulings, audit items). */
.card {
  position: relative;
  margin: 0 0 14px;
  padding: 16px 16px 12px;
  background: var(--paper-raised);
  border: 1px solid var(--paper-edge);
  border-radius: 6px;
  box-shadow: var(--shadow);
  scroll-margin-top: 16px;
}
.card.is-confirmed { border-color: color-mix(in srgb, var(--seal) 26%, var(--paper-edge)); }
.card.is-draft {
  background: repeating-linear-gradient(-45deg, transparent 0 9px, var(--ochre-wash) 9px 10px), var(--paper);
  border: 1px dashed color-mix(in srgb, var(--ochre) 55%, transparent);
  box-shadow: none;
}
.card:target { box-shadow: 0 0 0 3px var(--seal-wash), var(--shadow); border-color: var(--seal-line); }
.card__head { display: flex; align-items: baseline; gap: 10px; }
.card__id { flex: none; font: 15px/1.4 var(--font-num); letter-spacing: 0.06em; color: var(--ink-faint); }
.card.is-confirmed .card__id { color: var(--seal); }
.card.is-draft .card__id { color: var(--ochre); }
.card__title { font: 600 17px/1.45 var(--font-ui); letter-spacing: 0.02em; }
.card.is-draft .card__title { color: var(--ink-soft); }
.card__mark { margin-left: auto; flex: none; }
.stamp {
  display: inline-block;
  padding: 1px 6px;
  font: 700 12.5px/1.5 var(--font-serif);
  letter-spacing: 0.15em;
  color: var(--seal);
  border: 1.5px solid var(--seal);
  border-radius: 3px;
  transform: rotate(-4deg);
}
.badge {
  display: inline-block;
  padding: 0 8px;
  font-size: 12px;
  line-height: 1.8;
  color: var(--ochre);
  border: 1px dashed color-mix(in srgb, var(--ochre) 60%, transparent);
  border-radius: 3px;
  background: var(--ochre-wash);
}
.chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
.chip {
  font-size: 12.5px;
  line-height: 1.8;
  padding: 0 8px;
  color: var(--ink-soft);
  background: var(--paper-sunk);
  border-radius: 999px;
}
.chip.is-high { color: var(--seal-ink); background: var(--seal); }
.quote {
  margin: 12px 0 8px;
  padding: 2px 0 2px 14px;
  border-left: 2px solid var(--seal-line);
  font: 18px/1.85 var(--font-serif);
  letter-spacing: 0.03em;
}
.card.is-draft .quote { border-left-style: dashed; border-left-color: color-mix(in srgb, var(--ochre) 60%, transparent); }
.quote p { margin: 0.15em 0; }
.small { font-size: 14px; color: var(--ink-soft); }

/* Sections of the 底色. */
.section { margin-top: 30px; }
.section__head {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--rule);
}
.section__ordinal {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  flex: none;
  font: 700 16px/1 var(--font-serif);
  color: var(--seal-ink);
  background: var(--seal);
  border-radius: 3px;
}
.section__title { font: 700 19px/1.3 var(--font-serif); letter-spacing: 0.08em; }
.section__count { margin-left: auto; font-size: 12.5px; color: var(--ink-faint); }

/* Markdown. */
.prose { font-size: 15px; line-height: 1.8; }
.prose > * + * { margin-top: 0.6em; }
.prose h2, .prose h3, .prose h4 { font: 700 16px/1.5 var(--font-serif); letter-spacing: 0.06em; margin-top: 1.1em; }
.prose h2 { padding-bottom: 4px; border-bottom: 1px solid var(--rule); }
.prose > :first-child { margin-top: 0; }
.prose ul, .prose ol { padding-left: 1.3em; margin-bottom: 0; }
.prose li + li { margin-top: 0.2em; }
.prose blockquote {
  margin-left: 0;
  margin-right: 0;
  padding-left: 12px;
  border-left: 2px solid var(--seal-line);
  font-family: var(--font-serif);
  color: var(--ink-soft);
}
.prose code { font: 0.88em ui-monospace, Menlo, monospace; padding: 0 3px; background: var(--paper-sunk); border-radius: 3px; }
.prose pre { overflow-x: auto; padding: 10px; background: var(--paper-sunk); border-radius: 4px; }
.prose pre code { padding: 0; background: none; }
.prose table { display: block; overflow-x: auto; border-collapse: collapse; font-size: 14px; }
.prose th, .prose td { padding: 4px 8px; border: 1px solid var(--rule); text-align: left; vertical-align: top; }
.prose hr { border: 0; border-top: 1px solid var(--rule); }
.prose input[type="checkbox"] { margin: 0 6px 0 0; accent-color: var(--seal); vertical-align: -1px; }
.prose .contains-task-list { padding-left: 0; }
.prose .task-list-item { list-style: none; }
.plain { color: inherit; }

/* 今日. */
.today__date { font: 700 40px/1.15 var(--font-serif); letter-spacing: 0.08em; }
.today__entry { margin-top: 18px; }
.counts { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; margin: 18px 0; }
.count {
  display: block;
  padding: 12px 10px;
  text-decoration: none;
  color: inherit;
  background: var(--paper-raised);
  border: 1px solid var(--paper-edge);
  border-radius: 6px;
}
.count__n { display: block; font: 30px/1.1 var(--font-num); color: var(--seal); }
.count__label { display: block; margin-top: 4px; font-size: 13px; color: var(--ink-soft); }
.count__sub { display: block; font-size: 12px; color: var(--ink-faint); }
.links { list-style: none; padding: 0; margin: 0; border-top: 1px solid var(--rule); }
.links li { border-bottom: 1px solid var(--rule); }
.links a { display: flex; justify-content: space-between; gap: 12px; padding: 12px 2px; color: var(--ink); text-decoration: none; }
.links span { color: var(--ink-faint); font-size: 14px; }

/* Notes and boxes. */
.box {
  margin: 0 0 18px;
  padding: 12px 14px;
  font-size: 14.5px;
  background: var(--seal-wash);
  border: 1px solid var(--seal-line);
  border-radius: 6px;
}
.box h2 { font: 700 15px/1.5 var(--font-serif); letter-spacing: 0.08em; color: var(--seal); margin-bottom: 4px; }
.box ol { margin: 6px 0 0; padding-left: 1.3em; }
.empty { padding: 28px 0; text-align: center; color: var(--ink-faint); font-family: var(--font-serif); }
.toc { display: flex; flex-wrap: wrap; gap: 6px; margin: 0 0 18px; }
.toc a { font-size: 13px; padding: 1px 9px; border: 1px solid var(--paper-edge); border-radius: 999px; text-decoration: none; color: var(--ink-soft); background: var(--paper-raised); }
.audit__parts { margin-top: 6px; font-size: 14.5px; }
.audit__parts dt { float: left; clear: left; width: 3.2em; color: var(--ink-faint); }
.audit__parts dd { margin: 0 0 4px 3.4em; }
.foot { margin-top: 34px; padding-top: 12px; border-top: 1px solid var(--rule); font-size: 12.5px; color: var(--ink-faint); text-align: center; }
`.trim();
