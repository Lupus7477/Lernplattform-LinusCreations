// Baut web/icons.svg aus den SVG-Exporten von design/icons.vectorcraft.
//
// Ablauf: In VectorCraft ändern, alle Artboards als SVG exportieren
// (export, format svg, range "1-46"), dann:
//   node design/build-icons.mjs <exportordner>
//
// Konvention im VectorCraft-Dokument („Glanz-Sticker“), erkannt an der Kontur:
//   - Farbfläche: Füllung, keine Kontur. Im Ruhezustand zart getönt, aktiv/Hover satt (Farbe aus dem CSS).
//   - Linie: Kontur, fill="none". Farbe, Stärke und Enden kommen aus dem CSS (.icon in web/css/app.css).
//   - Glanzlicht: weiße Kontur. Nur sichtbar, wenn das Icon gefüllt ist (--icon-shine im CSS).
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2];
if (!dir) throw new Error("Aufruf: node design/build-icons.mjs <exportordner>");

const files = readdirSync(dir).filter((f) => f.endsWith(".svg")).sort();
const symbols = files.map((f) => {
  const svg = readFileSync(join(dir, f), "utf8");
  const name = svg.match(/<title>([^<]+)<\/title>/)?.[1]; // Artboard-Name
  const viewBox = svg.match(/viewBox="([^"]+)"/)?.[1];
  const shapes = (svg.match(/<(path|circle|ellipse|rect|line|polyline|polygon)\b[^>]*\/>/g) || []).map((el) => {
    const stroke = el.match(/\sstroke="([^"]*)"/)?.[1] ?? "none";
    const out = el
      .replace(/\s(id|data-name|style|stroke|stroke-[a-z]+|opacity)="[^"]*"/g, "")
      .replace(/\sfill="(?!none")[^"]*"/g, "")
      .replace(/\s*\/>$/, "");
    if (stroke === "none") return `${out} stroke="none"/>`; // Farbfläche
    if (/^#f{3}(f{3})?$/i.test(stroke)) return `${out} style="stroke:var(--icon-shine,none);stroke-width:1.5"/>`; // Glanzlicht
    return `${out}/>`; // Linie
  });
  if (!name || !viewBox || !shapes.length) throw new Error(`${f}: kein Titel, viewBox oder keine Form`);
  return `<symbol id="i-${name}" viewBox="${viewBox}">${shapes.join("")}</symbol>`;
});

writeFileSync(
  new URL("../web/icons.svg", import.meta.url),
  `<svg xmlns="http://www.w3.org/2000/svg" style="display:none">\n<!-- Lernpfad-Icons, Quelle: design/icons.vectorcraft, gebaut mit design/build-icons.mjs -->\n${symbols.join("\n")}\n</svg>\n`,
);
console.log(`${symbols.length} Icons → web/icons.svg`);
