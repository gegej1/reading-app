// Writes the iOS shortcuts of the iPhone bridge into shortcuts/ as XML
// property lists (the sources the app embeds and signs on the Mac), and
// optionally signs them into a folder:
//
//   node scripts/build-shortcuts.mjs                 # update shortcuts/*.plist
//   node scripts/build-shortcuts.mjs --check         # fail if they are out of date
//   node scripts/build-shortcuts.mjs --sign <dir>    # also sign into <dir> (macOS)
//
// Signing uses `plutil -convert binary1` and `shortcuts sign --mode anyone`;
// the app does the same by itself for the bridge folder (src-tauri/src/bridge/shortcuts.rs).

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { allShortcuts, toPlist } from "./shortcuts/definitions.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const sourceDir = join(root, "shortcuts");

const args = process.argv.slice(2);
const check = args.includes("--check");
const signIndex = args.indexOf("--sign");
const signDir = signIndex === -1 ? null : args[signIndex + 1];
if (signIndex !== -1 && !signDir) {
  console.error("--sign 后面要写一个文件夹");
  process.exit(2);
}

let stale = 0;
for (const [name, shortcut] of Object.entries(allShortcuts())) {
  const file = join(sourceDir, `${name}.plist`);
  const xml = toPlist(shortcut);
  let current = null;
  try {
    current = readFileSync(file, "utf8");
  } catch {
    current = null;
  }
  if (current !== xml) {
    stale += 1;
    if (check) {
      console.error(`过时了：shortcuts/${name}.plist（运行 node scripts/build-shortcuts.mjs）`);
    } else {
      mkdirSync(sourceDir, { recursive: true });
      writeFileSync(file, xml);
      console.log(`写好了 shortcuts/${name}.plist`);
    }
  }
  if (signDir) {
    const work = mkdtempSync(join(tmpdir(), "dise-shortcut-"));
    try {
      const binary = join(work, `${name}.shortcut`);
      const run = (program, programArgs) => {
        const result = spawnSync(program, programArgs, { encoding: "utf8" });
        if (result.status !== 0) throw new Error(`${program} failed: ${result.stderr}`);
      };
      writeFileSync(join(work, "source.plist"), xml);
      run("/usr/bin/plutil", ["-convert", "binary1", "-o", binary, join(work, "source.plist")]);
      mkdirSync(signDir, { recursive: true });
      const signed = join(signDir, `${name}.shortcut`);
      run("/usr/bin/shortcuts", ["sign", "--mode", "anyone", "--input", binary, "--output", signed]);
      if (readFileSync(signed).subarray(0, 4).toString("latin1") !== "AEA1") {
        throw new Error(`${name}: shortcuts sign wrote no signed file`);
      }
      console.log(`签好了 ${signed}`);
    } finally {
      rmSync(work, { recursive: true, force: true });
    }
  }
}
if (check && stale > 0) process.exit(1);
