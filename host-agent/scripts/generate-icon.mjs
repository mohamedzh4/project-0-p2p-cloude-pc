/*
 * Draws the source app icon so the mark is reproducible from code instead of a
 * checked-in binary nobody can edit. Run it, then hand the result to the Tauri
 * CLI which derives every platform size:
 *
 *   node scripts/generate-icon.mjs
 *   npm run tauri icon src-tauri/icons/source.png
 *
 * The mark is three linked nodes: one machine lending capacity to others.
 */

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 1024;
const SAMPLES = 4; // supersampling per axis, for smooth edges
const BACKGROUND = [47, 111, 104]; // --accent
const MARK = [238, 241, 245]; // --bg

const CORNER_RADIUS = SIZE * 0.22;
const CENTER = SIZE / 2;
const ORBIT = SIZE * 0.23;
const HUB_RADIUS = SIZE * 0.088;
const NODE_RADIUS = SIZE * 0.062;
const LINK_WIDTH = SIZE * 0.028;

const nodes = [0, 1, 2].map((index) => {
  const angle = -Math.PI / 2 + (index * 2 * Math.PI) / 3;
  return { x: CENTER + ORBIT * Math.cos(angle), y: CENTER + ORBIT * Math.sin(angle) };
});

function insideRoundedSquare(x, y) {
  const inset = SIZE * 0.02;
  const min = inset;
  const max = SIZE - inset;

  if (x < min || x > max || y < min || y > max) {
    return false;
  }

  const cx = Math.min(Math.max(x, min + CORNER_RADIUS), max - CORNER_RADIUS);
  const cy = Math.min(Math.max(y, min + CORNER_RADIUS), max - CORNER_RADIUS);

  return Math.hypot(x - cx, y - cy) <= CORNER_RADIUS;
}

function distanceToSegment(x, y, a, b) {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const lengthSquared = dx * dx + dy * dy;
  const t = Math.max(0, Math.min(1, ((x - a.x) * dx + (y - a.y) * dy) / lengthSquared));

  return Math.hypot(x - (a.x + t * dx), y - (a.y + t * dy));
}

function insideMark(x, y) {
  if (Math.hypot(x - CENTER, y - CENTER) <= HUB_RADIUS) {
    return true;
  }

  for (const node of nodes) {
    if (Math.hypot(x - node.x, y - node.y) <= NODE_RADIUS) {
      return true;
    }

    if (distanceToSegment(x, y, { x: CENTER, y: CENTER }, node) <= LINK_WIDTH / 2) {
      return true;
    }
  }

  return false;
}

/** Fraction of a pixel covered by `test`, sampled on a SAMPLES x SAMPLES grid. */
function coverage(px, py, test) {
  let hits = 0;

  for (let sy = 0; sy < SAMPLES; sy += 1) {
    for (let sx = 0; sx < SAMPLES; sx += 1) {
      if (test(px + (sx + 0.5) / SAMPLES, py + (sy + 0.5) / SAMPLES)) {
        hits += 1;
      }
    }
  }

  return hits / (SAMPLES * SAMPLES);
}

function render() {
  // One filter byte (0 = none) in front of every RGBA scanline.
  const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));

  for (let y = 0; y < SIZE; y += 1) {
    const rowStart = y * (SIZE * 4 + 1);
    raw[rowStart] = 0;

    for (let x = 0; x < SIZE; x += 1) {
      const square = coverage(x, y, insideRoundedSquare);
      const mark = coverage(x, y, insideMark) * square;
      const offset = rowStart + 1 + x * 4;

      for (let channel = 0; channel < 3; channel += 1) {
        raw[offset + channel] = Math.round(
          BACKGROUND[channel] * (1 - mark) + MARK[channel] * mark,
        );
      }
      raw[offset + 3] = Math.round(square * 255);
    }
  }

  return raw;
}

function crc32(buffer) {
  let crc = 0xffffffff;

  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }

  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);

  const typed = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(typed));

  return Buffer.concat([length, typed, checksum]);
}

function encodePng(raw) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(SIZE, 0);
  header.writeUInt32BE(SIZE, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  header[10] = 0; // deflate
  header[11] = 0; // adaptive filtering
  header[12] = 0; // no interlace

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const output = resolve(dirname(fileURLToPath(import.meta.url)), "../src-tauri/icons/source.png");
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, encodePng(render()));

console.log(`Wrote ${output}`);
