// Hinzufügen: Kurs anlegen, PDF einlesen (pdf.js, läuft in jeder WebView), Fortschritt der Vorbereitung.
import { call } from "./api.js";
import { confirmSheet, flip, fmtDate, h, icon, openSheet, segmented, toast } from "./ui.js";
import { aiNotice, currentCourse, emit, loadCourses, selectCourse, state, subscribe } from "./state.js";

let pdfjs;
/** Liest Text und Grafiken zum Beschriften. Eine Seite ohne erkennbare Grafik kostet nur das Operator-Lesen. */
async function readPdf(file, onPage) {
  pdfjs ??= await import("../vendor/pdf.min.mjs").then((m) => {
    m.GlobalWorkerOptions.workerSrc = new URL("../vendor/pdf.worker.min.mjs", import.meta.url).href;
    return m;
  });
  const task = pdfjs.getDocument({ data: new Uint8Array(await file.arrayBuffer()), isEvalSupported: false });
  const doc = await task.promise;
  const pages = [];
  const figures = [];
  for (let n = 1; n <= doc.numPages; n++) {
    const page = await doc.getPage(n);
    const content = await page.getTextContent();
    pages.push(content.items.map((it) => (it.str ?? "") + (it.hasEOL ? "\n" : " ")).join(""));
    if (figures.length < FIG.maxFigures) {
      try {
        const f = await findFigure(page, content);
        if (f) figures.push({ page: n, ...f });
      } catch (e) {
        console.warn(`Grafik auf Seite ${n} übersprungen:`, e);
      }
    }
    page.cleanup();
    onPage(n, doc.numPages, figures.length);
  }
  await task.destroy();
  return { pages, figures };
}

// ---------- Grafiken zum Beschriften (ohne KI: die Beschriftung steht als Text im PDF) ----------
// ponytail: Faustregeln für Folien-PDFs. Greifen sie daneben, hier nachjustieren; schlechte Aufgaben kann man ausblenden.
const FIG = { maxFigures: 30, maxSpots: 8, maxPx: 2000, minLabels: 3, bigImage: 0.25, near: 0.04 };
const mul = (m, n) => [m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1], m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3], m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]];
const at = (m, x, y) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
const bbox = (pts) => {
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
};
const union = (rs) => bbox(rs.flatMap((r) => [[r.x, r.y], [r.x + r.w, r.y + r.h]]));

/** Bilder und Formen der Seite als Rechtecke (Viewport-Koordinaten, y nach unten). Hintergründe und Kopflinien fallen weg. */
async function graphicBoxes(page, vp) {
  const { fnArray, argsArray } = await page.getOperatorList();
  const O = pdfjs.OPS;
  const W = vp.width;
  const H = vp.height;
  const out = [];
  let ctm = [1, 0, 0, 1, 0, 0];
  const stack = [];
  const add = (x0, y0, x1, y1, image) => {
    const r = bbox([at(ctm, x0, y0), at(ctm, x1, y0), at(ctm, x0, y1), at(ctm, x1, y1)].map(([x, y]) => at(vp.transform, x, y)));
    const share = (r.w * r.h) / (W * H);
    if (![r.x, r.y, r.w, r.h].every(Number.isFinite) || share > 0.85) return; // Folienhintergrund
    if (image && share < 0.01) return; // Logo, Symbol
    if (r.w > W * 0.9 && r.h < H * 0.03) return; // Linie über die ganze Breite
    out.push({ ...r, image });
  };
  const IMAGES = [O.paintImageXObject, O.paintInlineImageXObject, O.paintImageMaskXObject];
  for (let i = 0; i < fnArray.length; i++) {
    const fn = fnArray[i];
    const args = argsArray[i];
    if (fn === O.save) stack.push(ctm);
    else if (fn === O.restore) ctm = stack.pop() ?? ctm;
    else if (fn === O.transform) ctm = mul(ctm, args);
    else if (fn === O.paintFormXObjectBegin) {
      stack.push(ctm);
      if (args?.[0]?.length === 6) ctm = mul(ctm, args[0]);
    } else if (fn === O.paintFormXObjectEnd) ctm = stack.pop() ?? ctm;
    else if (IMAGES.includes(fn)) add(0, 0, 1, 1, true);
    else if (fn === O.constructPath) {
      const mm = args?.[args.length - 1]; // [minX, minY, maxX, maxY] des Pfads
      if (mm?.length === 4 && [...mm].every(Number.isFinite)) add(mm[0], mm[1], mm[2], mm[3], false);
    }
  }
  return out;
}

/** Häufigste Farbe am Rand eines Rechtecks: damit wird die Beschriftung übermalt. */
function edgeColor(ctx, p) {
  const m = 3;
  const x0 = Math.max(0, Math.floor(p.x - m));
  const y0 = Math.max(0, Math.floor(p.y - m));
  const x1 = Math.min(ctx.canvas.width, Math.ceil(p.x + p.w + m));
  const y1 = Math.min(ctx.canvas.height, Math.ceil(p.y + p.h + m));
  if (x1 <= x0 || y1 <= y0) return "#fff";
  const { data, width, height } = ctx.getImageData(x0, y0, x1 - x0, y1 - y0);
  const count = new Map();
  let best = [255, 255, 255];
  let top = 0;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (x >= m && x < width - m && y >= m && y < height - m) continue; // nur der Rand
      const i = (y * width + x) * 4;
      const key = ((data[i] >> 4) << 8) | ((data[i + 1] >> 4) << 4) | (data[i + 2] >> 4);
      const n = (count.get(key) ?? 0) + 1;
      count.set(key, n);
      if (n > top) [top, best] = [n, [data[i], data[i + 1], data[i + 2]]];
    }
  }
  return `rgb(${best.join(",")})`;
}

/** Ausschnitt rendern, gewählte Beschriftungen übermalen, Stellen relativ zum Ausschnitt (0–1). */
async function renderCrop(page, r, labels) {
  // Lange Seite bis 2000 px: kleine Grafiken bleiben beim Zoomen scharf, hohe werden nicht riesig.
  const scale = Math.min(4, FIG.maxPx / Math.max(r.w, r.h));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.ceil(r.w * scale));
  canvas.height = Math.max(1, Math.ceil(r.h * scale));
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  await page.render({ canvasContext: ctx, viewport: page.getViewport({ scale }), transform: [1, 0, 0, 1, -r.x * scale, -r.y * scale], background: "#fff" }).promise;
  const clamp01 = (v) => Math.min(1, Math.max(0, v));
  const spots = labels.map((l) => {
    const pad = l.h * 0.15;
    const box = { x: l.x - pad, y: l.y - pad, w: l.w + 2 * pad, h: l.h + 2 * pad };
    const px = { x: (box.x - r.x) * scale, y: (box.y - r.y) * scale, w: box.w * scale, h: box.h * scale };
    ctx.fillStyle = edgeColor(ctx, px);
    ctx.fillRect(px.x, px.y, px.w, px.h);
    const x = clamp01((box.x - r.x) / r.w);
    const y = clamp01((box.y - r.y) / r.h);
    return { x, y, w: Math.min(box.w / r.w, 1 - x), h: Math.min(box.h / r.h, 1 - y), label: l.str };
  });
  const webp = canvas.toDataURL("image/webp", 0.82);
  let image = webp.startsWith("data:image/webp") ? webp : canvas.toDataURL("image/jpeg", 0.82);
  if (image.length > 1_200_000) image = canvas.toDataURL("image/jpeg", 0.6);
  canvas.width = canvas.height = 0; // Speicher sofort freigeben (Handy)
  return image.length > 1_900_000 ? null : { image, spots };
}

/**
 * Grafik einer Seite: kurze Texte (Beschriftungen) nahe an Bildern oder Formen → Ausschnitt mit übermalten Beschriftungen.
 * Große Bilder ohne Text (Karte, Foto) kommen ohne Stellen mit, für eigene Aufgaben. Sonst null.
 */
async function findFigure(page, content) {
  const vp = page.getViewport({ scale: 1 });
  const W = vp.width;
  const H = vp.height;
  const boxes = await graphicBoxes(page, vp);
  if (!boxes.length) return null;
  const items = content.items
    .filter((it) => it.str?.trim())
    .map((it) => {
      const t = mul(vp.transform, it.transform);
      const size = Math.hypot(t[2], t[3]);
      const str = it.str.trim();
      return { str, size, x: t[4], y: t[5] - size * 0.92, w: it.width * vp.scale || str.length * size * 0.5, h: size * 1.22 };
    });
  const sizes = items.map((i) => i.size).sort((a, b) => a - b);
  const median = sizes[Math.floor(sizes.length / 2)] ?? 0;
  const title = items.filter((i) => i.y < H * 0.3).sort((a, b) => b.size - a.size)[0];
  const near = (it, b) => {
    const m = W * FIG.near;
    const cx = it.x + it.w / 2;
    const cy = it.y + it.h / 2;
    return cx >= b.x - m && cx <= b.x + b.w + m && cy >= b.y - m && cy <= b.y + b.h + m;
  };
  const labels = items.filter(
    (it) =>
      it !== title &&
      it.str.length >= 2 &&
      it.str.length <= 40 &&
      it.str.split(/\s+/).length <= 4 &&
      /\p{L}/u.test(it.str) &&
      !/[.!?;]$/.test(it.str) &&
      it.size <= median * 2 + 0.01 && // Feldtitel sind oft fett und größer als Fließtext …
      (!title || it.size < title.size * 0.9) && // … aber kleiner als der Folientitel
      it.y > H * 0.08 &&
      it.y + it.h < H * 0.93 &&
      boxes.some((b) => near(it, b)),
  );
  const heading = title?.str.slice(0, 80) ?? "";
  // Ohne echtes Bild nur, wenn die Beschriftungen über mind. 2 Spalten verteilt sind: sonst ist es eine Aufzählung im Kasten.
  const columns = new Set(labels.map((l) => Math.round(l.x / (W * 0.08)))).size;
  if (columns < 2 && !boxes.some((b) => b.image && labels.some((l) => near(l, b)))) labels.length = 0;
  if (labels.length < FIG.minLabels) {
    const big = boxes.filter((b) => b.image && b.w * b.h >= W * H * FIG.bigImage).sort((a, b) => b.w * b.h - a.w * a.h)[0];
    const crop = big && (await renderCrop(page, big, []));
    return crop ? { title: heading, ...crop } : null;
  }
  // Höchstens 8 Stellen, über die Fläche verteilt
  const sorted = [...labels].sort((a, b) => a.y - b.y || a.x - b.x);
  const step = Math.ceil(sorted.length / FIG.maxSpots);
  const chosen = sorted.filter((_, i) => i % step === 0).slice(0, FIG.maxSpots);
  let r = union([...boxes.filter((b) => chosen.some((l) => near(l, b))), ...chosen]);
  // Formen, die in den Ausschnitt ragen (Pfeile, Linien, Rahmen), ganz mitnehmen, sonst ist die Grafik abgeschnitten.
  // Große Flächen (Hintergrund-Kästen) nicht: die würden den Ausschnitt auf die halbe Folie aufblähen.
  const overlaps = (a, b) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
  for (let k = 0; k < 3; k++) {
    const grown = union([r, ...boxes.filter((b) => overlaps(b, r) && b.w * b.h <= r.w * r.h * 1.5)]);
    if (grown.w * grown.h <= r.w * r.h + 1) break;
    r = grown;
  }
  const m = W * 0.03;
  // Rand um die Grafik, aber nicht in den Folientitel darüber hinein
  const top = Math.max(0, r.y - m, title && title.y + title.h <= r.y ? title.y + title.h : 0);
  const region = { x: Math.max(0, r.x - m), y: top, w: Math.min(W, r.x + r.w + m) - Math.max(0, r.x - m), h: Math.min(H, r.y + r.h + m) - top };
  const crop = await renderCrop(page, region, chosen);
  return crop ? { title: heading, ...crop } : null;
}

export async function render() {
  const page = h("div.page");
  const draw = () => page.replaceChildren(...content(page));
  draw();
  page.cleanup = subscribe((kind) => {
    if (document.querySelector(".doc.is-dragging")) return; // nicht mitten im Ziehen neu zeichnen
    if (kind === "course-updated" || kind === "courses" || kind === "ai") draw();
  });
  return page;
}

function content(page) {
  // PDFs nur in eigene Kurse; geteilte Kurse gehören dem, der sie angelegt hat.
  const courses = state.courses.filter((c) => c.mine);
  const selected = currentCourse()?.mine ? currentCourse() : courses[0];
  const status = h("div.stack", { role: "status", "aria-live": "polite" });

  const newName = h("input.input", { id: "course-name", required: true, autocomplete: "off", placeholder: "z. B. Mikroökonomie" });
  const newDate = h("input.input", { id: "course-date", type: "date" });
  const newFields = h(
    "div.stack",
    h("div.field", h("label", { for: "course-name" }, "Name des Kurses"), newName),
    h("div.field", h("label", { for: "course-date" }, "Prüfungsdatum (optional)"), newDate, h("span.hint", "Damit siehst du, wie viel du pro Tag schaffen solltest.")),
  );

  let pick = null;
  if (courses.length) {
    pick = h(
      "select.input",
      { id: "course-pick", onchange: () => (newFields.hidden = pick.value !== "new") },
      courses.map((c) => h("option", { value: String(c.id), selected: c.id === selected?.id }, c.name)),
      h("option", { value: "new" }, "Neuer Kurs …"),
    );
    newFields.hidden = true;
  }

  const input = h("input", { type: "file", accept: "application/pdf,.pdf", multiple: true, hidden: true });
  const drop = h(
    "label.drop",
    { tabindex: "0", onkeydown: (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), input.click()) },
    icon("file-pdf"),
    h("b", "PDF auswählen"),
    h("span.muted", "Vorlesungsskripte, Folien oder Übungsblätter. Mehrere auf einmal gehen auch."),
    input,
  );
  drop.addEventListener("dragover", (e) => (e.preventDefault(), drop.classList.add("over")));
  drop.addEventListener("dragleave", () => drop.classList.remove("over"));
  drop.addEventListener("drop", (e) => {
    e.preventDefault();
    drop.classList.remove("over");
    importFiles([...e.dataTransfer.files]);
  });
  input.addEventListener("change", () => importFiles([...input.files]));

  async function targetCourse() {
    if (pick && pick.value !== "new") return Number(pick.value);
    const name = newName.value.trim();
    if (!name) {
      newName.focus();
      status.replaceChildren(h("p", { role: "alert", style: { color: "var(--wrong)", "font-weight": "600" } }, "Gib dem Kurs zuerst einen Namen."));
      return null;
    }
    return call("create_course", { name, examDate: newDate.value || null });
  }

  async function importFiles(files) {
    const pdfs = files.filter((f) => /\.pdf$/i.test(f.name) || f.type === "application/pdf");
    input.value = "";
    if (!pdfs.length) return;
    const courseId = await targetCourse();
    if (courseId == null) return;
    for (const file of pdfs) {
      const line = h("span.muted.num", "Wird geöffnet …");
      const bar = h("i", { style: { "--p": "0" } });
      status.append(h("div.card", h("b", { style: { "overflow-wrap": "anywhere" } }, file.name), line, h("div.bar", bar)));
      try {
        const { pages, figures } = await readPdf(file, (n, of, figs) => {
          line.textContent = `Seite ${n} von ${of} gelesen${figs ? ` · ${figs} ${figs === 1 ? "Grafik" : "Grafiken"} gefunden` : ""}`;
          bar.style.setProperty("--p", String(n / of));
        });
        const res = await call("import_document", { courseId, name: file.name, pages, figures });
        line.textContent = `${res.units} ${res.units === 1 ? "Einheit wird" : "Einheiten werden"} vorbereitet${res.figures ? `, ${res.figures} ${res.figures === 1 ? "Grafik" : "Grafiken"} zum Beschriften` : ""}.${skippedNote(res)}`;
      } catch (e) {
        line.textContent = String(e?.message || e);
        line.style.color = "var(--wrong)";
      }
    }
    await loadCourses();
    selectCourse(courseId);
    emit("courses");
    toast("Fertig eingelesen. Die ersten Lektionen erscheinen gleich im Lernpfad.");
  }

  return [
    h("h1", "Skript hinzufügen"),
    aiNotice(),
    h("div.card", pick ? h("div.field", h("label", { for: "course-pick" }, "Kurs"), pick) : h("h2", "Neuer Kurs"), newFields, drop, status),
    state.courses.length ? h("h2", "Deine Kurse") : null,
    ...state.courses.map((c) => courseCard(c, page)),
  ].filter(Boolean);
}

const skippedNote = (r) => (r.skipped ? ` ${r.skipped} doppelte ${r.skipped === 1 ? "Seite" : "Seiten"} übersprungen.` : "");
const fail = (e) => toast(String(e?.message || e));
async function refresh() {
  await loadCourses();
  emit("courses");
  emit("course-updated", null);
}

function courseCard(c) {
  const done = c.ready === c.units;
  const badge = !c.mine ? h("span.badge", `von ${c.owner}`) : c.share_all ? h("span.badge", "Mit allen geteilt") : c.members ? h("span.badge", `Geteilt mit ${c.members}`) : null;
  // Eigene PDFs: sortieren (ab zwei), ersetzen (neue Fassung) oder löschen, beides mit Rückfrage
  const n = c.documents.length;
  const doc = (d, i) =>
    c.mine
      ? h(
          "div.doc",
          { "data-id": String(d.id) },
          n > 1 ? h("button.icon-btn.grip", { type: "button", title: "Ziehen oder mit ↑ ↓ verschieben", "aria-label": `„${d.name}“ verschieben, Stelle ${i + 1} von ${n}` }, icon("dots-six-vertical")) : null,
          icon("file-pdf"),
          h("span.name", d.name),
          h("button.icon-btn", { type: "button", title: "Ersetzen", "aria-label": `„${d.name}“ ersetzen`, onclick: () => replaceDoc(d) }, icon("upload-simple")),
          h("button.icon-btn.del", { type: "button", title: "Löschen", "aria-label": `„${d.name}“ löschen`, onclick: () => deleteDoc(c, d) }, icon("trash")),
        )
      : h("span", icon("file-pdf"), d.name);
  return h(
    "div.card.course-card",
    h(
      "div.row.between",
      h("h3", c.name, badge),
      c.mine
        ? h("button.btn.ghost", { type: "button", onclick: () => editCourse(c) }, icon("pencil-simple"), "Bearbeiten")
        : c.can_invite
          ? h("button.btn.ghost", { type: "button", onclick: () => inviteSheet(c) }, icon("users"), "Personen hinzufügen")
          : null,
    ),
    c.exam_date ? h("p.muted", `Prüfung am ${fmtDate(c.exam_date)}`) : null,
    c.mine && n > 1 ? sortable(h("div.docs", c.documents.map(doc)), (ids, id, keyboard) => reorder(c, ids, id, keyboard)) : h("div.docs", c.documents.map(doc)),
    c.mine && n > 1 ? h("p.hint", "In dieser Reihenfolge kommen die PDFs im Lernpfad.") : null,
    h("div.bar", h("i", { style: { "--p": String(c.units ? c.ready / c.units : 0) } })),
    h("p.muted.num", done ? `Alle ${c.units} Einheiten bereit` : `${c.ready} von ${c.units} Einheiten vorbereitet`),
  );
}

/**
 * Zeilen am Griff ziehen (Maus, Finger) oder per Pfeiltaste am Griff verschieben.
 * Die gezogene Zeile folgt dem Zeiger, die anderen weichen per FLIP aus. done(ids, id, keyboard) nur bei geänderter Folge.
 */
function sortable(list, done) {
  const ids = () => [...list.children].map((r) => Number(r.dataset.id));
  list.addEventListener("pointerdown", (e) => {
    const grip = e.target.closest(".grip");
    if (!grip || e.button > 0) return;
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const row = grip.closest(".doc");
    const before = ids().join();
    const y0 = e.clientY;
    const top0 = row.offsetTop;
    row.classList.add("is-dragging");
    const move = (ev) => {
      // Layout-Positionen (offsetTop) statt getBoundingClientRect: laufende FLIP-Animationen verfälschen sie nicht.
      const want = top0 + ev.clientY - y0;
      const others = [...list.children].filter((r) => r !== row);
      const next = others.find((r) => r.offsetTop + r.offsetHeight / 2 > want + row.offsetHeight / 2) ?? null;
      if (row.nextElementSibling !== next) flip(others, () => list.insertBefore(row, next), 200);
      row.style.transform = `translateY(${want - row.offsetTop}px)`;
    };
    const up = () => {
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", up);
      grip.removeEventListener("pointercancel", up);
      flip([row], () => (row.style.transform = ""), 200);
      row.classList.remove("is-dragging");
      if (ids().join() !== before) done(ids(), row.dataset.id, false);
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
    grip.addEventListener("pointercancel", up);
  });
  list.addEventListener("keydown", (e) => {
    const grip = e.target.closest(".grip");
    if (!grip || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    const row = grip.closest(".doc");
    const other = e.key === "ArrowUp" ? row.previousElementSibling : row.nextElementSibling;
    if (!other) return;
    flip([...list.children], () => (e.key === "ArrowUp" ? other.before(row) : other.after(row)), 200);
    grip.focus();
    done(ids(), row.dataset.id, true);
  });
  return list;
}

async function reorder(c, ids, id, keyboard) {
  try {
    await call("reorder_documents", { courseId: c.id, documentIds: ids });
  } catch (e) {
    fail(e);
  }
  await refresh();
  // Tastatur: Fokus bleibt am Griff, der Screenreader liest die neue Stelle vor
  if (keyboard) document.querySelector(`.doc[data-id="${id}"] .grip`)?.focus();
}

/** Neue Fassung eines PDFs: gleiche Abschnitte bleiben samt Fortschritt, bei geänderten prüft die KI die Fragen. */
function replaceDoc(d) {
  const input = h("input", { type: "file", accept: "application/pdf,.pdf" });
  input.addEventListener("change", async () => {
    const file = input.files[0];
    if (!file) return;
    const ok = await confirmSheet({
      title: "PDF ersetzen?",
      text: `„${file.name}“ ersetzt „${d.name}“. Unveränderte Abschnitte behalten Fragen und Fortschritt. Bei geänderten prüft die KI jede Frage: Veraltete fliegen raus, neue kommen dazu.`,
      confirm: "Ersetzen",
    });
    if (!ok) return;
    toast(`„${file.name}“ wird gelesen …`);
    try {
      const { pages, figures } = await readPdf(file, () => {});
      const r = await call("replace_document", { documentId: d.id, name: file.name, pages, figures });
      const parts = [r.changed && `${r.changed} geändert`, r.units && `${r.units} neu`, r.removed && `${r.removed} entfernt`].filter(Boolean);
      const text = parts.length
        ? `Ersetzt: ${parts.join(", ")}.${r.changed ? " Die KI prüft die Fragen." : ""}`
        : r.figures
          ? `Text unverändert, ${r.figures} ${r.figures === 1 ? "Grafik" : "Grafiken"} neu eingelesen.`
          : "Inhalt unverändert. Fragen und Fortschritt bleiben.";
      toast(text + skippedNote(r));
      await refresh();
    } catch (e) {
      fail(e);
    }
  });
  input.click();
}

async function deleteDoc(c, d) {
  const ok = await confirmSheet({
    title: `„${d.name}“ löschen?`,
    text: `Alle Einheiten und Fragen aus diesem PDF werden gelöscht, mit deinem Fortschritt darin${c.shared ? " und dem aller, mit denen du den Kurs teilst" : ""}. Das lässt sich nicht rückgängig machen.`,
    confirm: "Endgültig löschen",
    danger: true,
  });
  if (!ok) return;
  try {
    await call("delete_document", { documentId: d.id });
  } catch (e) {
    return fail(e);
  }
  await refresh();
  toast("PDF gelöscht.");
}

/** Personenliste mit Suche, Angehakte zuerst. `locked`: wer schon drin ist, bleibt angehakt (Mitglieder können nur hinzufügen). */
function peoplePicker(people, { locked = false } = {}) {
  if (!people.length) return { el: h("p.muted", "Noch niemand sonst auf dem Server."), picked: () => [] };
  const picked = new Set(people.filter((p) => p.member).map((p) => p.id));
  const search = h("input.input", { type: "search", autocomplete: "off", placeholder: "Name suchen", "aria-label": "Personen suchen" });
  const count = h("span.hint", { "aria-live": "polite" });
  const counted = () => (count.textContent = `${picked.size} ausgewählt`);
  const rows = [...people]
    .sort((a, b) => Number(b.member) - Number(a.member))
    .map((p) => {
      const box = h("input", { type: "checkbox", checked: p.member, disabled: locked && p.member });
      box.addEventListener("change", () => (box.checked ? picked.add(p.id) : picked.delete(p.id), counted()));
      return { name: p.name.toLowerCase(), el: h("label.person", box, h("span", p.name)) };
    });
  const empty = h("p.muted", { hidden: true }, "Niemand heißt so.");
  search.addEventListener("input", () => {
    const q = search.value.trim().toLowerCase();
    rows.forEach((r) => (r.el.hidden = !r.name.includes(q)));
    empty.hidden = rows.some((r) => !r.el.hidden);
  });
  counted();
  return { el: h("div.stack", search, h("div.people", rows.map((r) => r.el), empty), count), picked: () => [...picked] };
}

/** Mitglied mit Einladerecht: weitere Personen hinzufügen. */
async function inviteSheet(c) {
  let people;
  try {
    people = await call("share_people", { courseId: c.id });
  } catch (e) {
    return fail(e);
  }
  const picker = peoplePicker(people, { locked: true });
  const save = h("button.btn.block", { type: "button", disabled: !people.length }, "Hinzufügen");
  const s = openSheet({ title: "Personen hinzufügen", body: [h("p.muted", `Sie können mit „${c.name}“ lernen und Fragen verbessern. Ihr Fortschritt bleibt ihrer.`), picker.el], foot: save });
  save.addEventListener("click", async () => {
    const added = picker.picked().filter((id) => !people.find((p) => p.id === id)?.member).length;
    save.disabled = true;
    try {
      await call("invite_to_course", { courseId: c.id, userIds: picker.picked() });
    } catch (e) {
      save.disabled = false;
      return fail(e);
    }
    s.close();
    await refresh();
    toast(added ? `${added} ${added === 1 ? "Person" : "Personen"} hinzugefügt.` : "Keine Änderung.");
  });
}

const SHARE_HINT = {
  none: "Nur du siehst diesen Kurs.",
  some: "Die ausgewählten Personen können mit diesem Kurs lernen und Fragen verbessern. Dein Fortschritt bleibt deiner.",
  all: "Alle Konten auf diesem Server können mit diesem Kurs lernen und Fragen verbessern. Dein Fortschritt bleibt deiner.",
};

function editCourse(c) {
  const name = h("input.input", { id: "edit-name", value: c.name, autocomplete: "off" });
  const date = h("input.input", { id: "edit-date", type: "date", value: c.exam_date || "" });
  // Teilen gibt es nur auf dem eigenen Server: nur ich, ausgewählte Personen oder alle Konten.
  const onServer = state.session?.mode === "server";
  let mode = c.share_all ? "all" : c.members ? "some" : "none";
  let picker = null;
  const invite = h("input.switch", { type: "checkbox", id: "edit-invite", checked: c.members_invite });
  const hint = h("span.hint");
  const slot = h("div.stack");
  async function showMode(m) {
    mode = m;
    hint.textContent = SHARE_HINT[m];
    slot.hidden = m !== "some";
    if (m !== "some" || picker) return;
    slot.replaceChildren(h("div.skeleton", { style: { height: "88px" } }));
    try {
      picker = peoplePicker(await call("share_people", { courseId: c.id }));
      slot.replaceChildren(picker.el, h("div.row.between", h("label", { for: "edit-invite" }, "Mitglieder dürfen weitere Personen einladen"), invite));
    } catch (e) {
      slot.replaceChildren(h("p.muted", String(e?.message || e)));
    }
  }
  const share = onServer
    ? h("div.field", h("label", "Teilen"), segmented([["none", "Nur ich"], ["some", "Ausgewählte"], ["all", "Alle"]], mode, showMode, "Teilen"), hint, slot)
    : null;
  if (onServer) showMode(mode);
  const save = h("button.btn.block", { type: "button" }, "Speichern");
  const del = h("button.btn.ghost", { type: "button", style: { color: "var(--wrong)" } }, icon("trash"), "Kurs löschen");
  const s = openSheet({
    title: "Kurs bearbeiten",
    body: [h("div.field", h("label", { for: "edit-name" }, "Name"), name), h("div.field", h("label", { for: "edit-date" }, "Prüfungsdatum"), date), share, del],
    foot: save,
  });
  save.addEventListener("click", async () => {
    if (!name.value.trim()) return name.focus();
    save.disabled = true;
    try {
      await call("update_course", {
        id: c.id,
        name: name.value,
        examDate: date.value || null,
        ...(onServer ? { share: mode, members: picker?.picked(), membersInvite: picker ? invite.checked : undefined } : {}),
      });
    } catch (e) {
      save.disabled = false;
      return fail(e);
    }
    await loadCourses();
    emit("course-updated", c.id);
    emit("courses");
    s.close();
    toast("Gespeichert.");
  });
  del.addEventListener("click", async () => {
    s.close();
    const ok = await confirmSheet({
      title: `„${c.name}“ löschen?`,
      text: "Alle Einheiten, Fragen und dein Fortschritt in diesem Kurs werden gelöscht. Das lässt sich nicht rückgängig machen.",
      confirm: "Endgültig löschen",
      danger: true,
    });
    if (!ok) return;
    await call("delete_course", { id: c.id });
    await loadCourses();
    emit("course-updated", c.id);
    emit("courses");
    toast("Kurs gelöscht.");
  });
}
