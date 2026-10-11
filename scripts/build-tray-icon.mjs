// Builds the menu-bar (tray) icon: a monochrome template image of a seal
// impression (白文印), drawn from simple shapes only (rounded rectangles; no
// text, no font glyphs): a solid seal turned slightly, as if stamped by hand,
// with a carved frame and three carved bars that widen towards the base (底),
// echoing the app icon. macOS tints template images itself, black on light
// menu bars and white on dark ones, so only the alpha channel matters.
//
//   pnpm icon:tray
//
// Writes src-tauri/icons/tray-icon.svg (the shapes, for reference) and
// src-tauri/icons/tray-icon.png (36 × 36 px: 18 pt at 2×, the height macOS
// gives menu-bar icons). The PNG is rasterized right here, with 16 × 16
// samples per pixel, so no image tools are needed.

import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const iconsDir = join(root, "src-tauri", "icons");

// Drawn on an 18 × 18 pt canvas.
const CANVAS = 18;
const SCALE = 2;
const SAMPLES = 16;

// The seal, turned like the one on the app icon.
const seal = { cx: 9, cy: 9, size: 14.6, radius: 2.4, angle: -7 };
// The carved frame: its distance from the seal's edge and its width.
const FRAME_INSET = 1.3;
const FRAME_WIDTH = 1.0;
// Three carved bars, left-aligned, widening towards the base.
const BARS_LEFT = -3.9;
const BARS = [
  { y: -3.1, width: 3.6 },
  { y: 0, width: 5.4 },
  { y: 3.1, width: 7.2 },
];
const BAR_HEIGHT = 1.7;
const BAR_RADIUS = 0.5;

/** Signed distance from (px, py) to a rounded box of half sizes hw, hh (negative inside). */
function roundedBox(px, py, hw, hh, r) {
  const qx = Math.abs(px) - (hw - r);
  const qy = Math.abs(py) - (hh - r);
  const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0));
  return outside + Math.min(Math.max(qx, qy), 0) - r;
}

/** True when a point (canvas units) is inked. */
function inked(x, y) {
  // Into the seal's own frame: centered and turned back.
  const angle = (-seal.angle * Math.PI) / 180;
  const dx = x - seal.cx;
  const dy = y - seal.cy;
  const sx = dx * Math.cos(angle) - dy * Math.sin(angle);
  const sy = dx * Math.sin(angle) + dy * Math.cos(angle);

  const half = seal.size / 2;
  if (roundedBox(sx, sy, half, half, seal.radius) > 0) return false;
  const frameOuter = half - FRAME_INSET;
  const frameInner = frameOuter - FRAME_WIDTH;
  const inFrame =
    roundedBox(sx, sy, frameOuter, frameOuter, seal.radius - FRAME_INSET * 0.75) <= 0 &&
    roundedBox(sx, sy, frameInner, frameInner, seal.radius - (FRAME_INSET + FRAME_WIDTH) * 0.65) > 0;
  const inBar = BARS.some(
    (bar) =>
      roundedBox(sx - (BARS_LEFT + bar.width / 2), sy - bar.y, bar.width / 2, BAR_HEIGHT / 2, BAR_RADIUS) <= 0,
  );
  // Carved parts are where the seal leaves no ink.
  return !inFrame && !inBar;
}

function rasterize() {
  const size = CANVAS * SCALE;
  const alpha = new Uint8Array(size * size);
  for (let py = 0; py < size; py += 1) {
    for (let px = 0; px < size; px += 1) {
      let hits = 0;
      for (let sy = 0; sy < SAMPLES; sy += 1) {
        for (let sx = 0; sx < SAMPLES; sx += 1) {
          const x = (px + (sx + 0.5) / SAMPLES) / SCALE;
          const y = (py + (sy + 0.5) / SAMPLES) / SCALE;
          if (inked(x, y)) hits += 1;
        }
      }
      alpha[py * size + px] = Math.round((255 * hits) / (SAMPLES * SAMPLES));
    }
  }
  return { size, alpha };
}

// ---------- PNG encoding (RGBA, 8 bits per channel) ----------

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, "ascii");
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

function png({ size, alpha }) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  const rows = Buffer.alloc(size * (1 + size * 4));
  for (let y = 0; y < size; y += 1) {
    const row = y * (1 + size * 4);
    rows[row] = 0; // no filter
    for (let x = 0; x < size; x += 1) rows[row + 1 + x * 4 + 3] = alpha[y * size + x];
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(rows, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---------- SVG (the same shapes, for reference) ----------

function svg() {
  const half = seal.size / 2;
  const r = (x, y, w, h, radius, extra = "") =>
    `<rect x="${x.toFixed(2)}" y="${y.toFixed(2)}" width="${w.toFixed(2)}" height="${h.toFixed(2)}" rx="${radius}"${extra}/>`;
  const bars = BARS.map((bar) => r(BARS_LEFT, bar.y - BAR_HEIGHT / 2, bar.width, BAR_HEIGHT, BAR_RADIUS)).join(
    "\n        ",
  );
  const frame = half - FRAME_INSET - FRAME_WIDTH / 2;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${CANVAS}" height="${CANVAS}" viewBox="0 0 ${CANVAS} ${CANVAS}">
  <!-- 底色 menu-bar icon (template image): a seal with a carved frame and three carved bars, simple shapes only. -->
  <!-- Generated by scripts/build-tray-icon.mjs; edit the script, then run: pnpm icon:tray -->
  <defs>
    <mask id="carving">
      <g transform="translate(${seal.cx} ${seal.cy}) rotate(${seal.angle})">
        ${r(-half, -half, seal.size, seal.size, seal.radius, ' fill="#fff"')}
        ${r(-frame, -frame, frame * 2, frame * 2, seal.radius - FRAME_INSET, ` fill="none" stroke="#000" stroke-width="${FRAME_WIDTH}"`)}
        <g fill="#000">
        ${bars}
        </g>
      </g>
    </mask>
  </defs>
  <rect width="${CANVAS}" height="${CANVAS}" fill="#000" mask="url(#carving)"/>
</svg>
`;
}

const image = rasterize();
writeFileSync(join(iconsDir, "tray-icon.png"), png(image));
writeFileSync(join(iconsDir, "tray-icon.svg"), svg());
const inkedPixels = Array.from(image.alpha).filter((a) => a > 128).length;
console.log(`wrote src-tauri/icons/tray-icon.png (${image.size} × ${image.size}, ${inkedPixels} inked pixels) and tray-icon.svg`);
