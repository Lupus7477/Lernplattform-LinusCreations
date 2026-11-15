// Baut web/icons.svg aus den SVG-Exporten von design/icons.vectorcraft.
//
// Ablauf: In VectorCraft ändern, alle Artboards als SVG exportieren
// (export, format svg, range "1-46"), dann:
//   node design/build-icons.mjs <exportordner>
//
// Konvention im VectorCraft-Dokument: Flächen mit Farbe werden beim Hover gefüllt,
// Linien mit fill="none" bleiben Linien. Farbe, Strichstärke und Enden kommen aus
// dem CSS (.icon in web/css/app.css), darum fliegen sie hier raus.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2];
if (!dir) throw new Error("Aufruf: node design/build-icons.mjs <exportordner>");

const files = readdirSync(dir).filter((f) => f.endsWith(".svg")).sort();
const symbols = files.map((f) => {
  const svg = readFileSync(join(dir, f), "utf8");
  const name = svg.match(/<title>([^<]+)<\/title>/)?.[1]; // Artboard-Name
  const viewBox = svg.match(/viewBox="([^"]+)"/)?.[1];
  const shapes = (svg.match(/<(path|circle|ellipse|rect|line|polyline|polygon)\b[^>]*\/>/g) || []).map((el) =>
    el
      .replace(/\s(id|data-name|style|stroke|stroke-[a-z]+|opacity)="[^"]*"/g, "")
      .replace(/\sfill="(?!none")[^"]*"/g, "")
      .replace(/\s*\/>$/, "/>"),
  );
  if (!name || !viewBox || !shapes.length) throw new Error(`${f}: kein Titel, viewBox oder keine Form`);
  return `<symbol id="i-${name}" viewBox="${viewBox}">${shapes.join("")}</symbol>`;
});

writeFileSync(
  new URL("../web/icons.svg", import.meta.url),
  `<svg xmlns="http://www.w3.org/2000/svg" style="display:none">\n<!-- Lernpfad-Icons, Quelle: design/icons.vectorcraft, gebaut mit design/build-icons.mjs -->\n${symbols.join("\n")}\n</svg>\n`,
);
console.log(`${symbols.length} Icons → web/icons.svg`);
