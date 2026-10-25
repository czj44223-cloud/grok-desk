import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const size = 1024;
const pixels = Buffer.alloc(size * size * 4);
const blue = [29, 78, 216, 255];
const white = [255, 255, 255, 255];

function setPixel(x, y, color) {
  if (x < 0 || y < 0 || x >= size || y >= size) {
    return;
  }
  const offset = (y * size + x) * 4;
  pixels[offset] = color[0];
  pixels[offset + 1] = color[1];
  pixels[offset + 2] = color[2];
  pixels[offset + 3] = color[3];
}

for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    const dx = x - 512;
    const dy = y - 530;
    const distance = Math.hypot(dx, dy);
    const ring = distance <= 340 && distance >= 220;
    const angle = Math.atan2(dy, dx);
    const gap = angle > -0.95 && angle < 0.45;
    const bar = x >= 470 && x <= 820 && y >= 488 && y <= 572 && distance <= 340;
    setPixel(x, y, (ring && !gap) || bar ? white : blue);
  }
}

function chunk(type, data) {
  const body = Buffer.concat([Buffer.from(type), data]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(body.length - 4);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(body));
  return Buffer.concat([length, body, crc]);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(size, 0);
ihdr.writeUInt32BE(size, 4);
ihdr[8] = 8;
ihdr[9] = 6;
const raw = Buffer.alloc((size * 4 + 1) * size);
for (let y = 0; y < size; y += 1) {
  const row = y * (size * 4 + 1);
  raw[row] = 0;
  pixels.copy(raw, row + 1, y * size * 4, (y + 1) * size * 4);
}
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", zlib.deflateSync(raw)),
  chunk("IEND", Buffer.alloc(0)),
]);

const here = path.dirname(fileURLToPath(import.meta.url));
const target = path.join(here, "..", "src-tauri", "app-icon.png");
fs.writeFileSync(target, png);
console.log(target);
