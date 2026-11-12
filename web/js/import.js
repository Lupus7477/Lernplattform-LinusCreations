// Hinzufügen: Kurs anlegen, PDF einlesen (pdf.js, läuft in jeder WebView), Fortschritt der Vorbereitung.
import { call } from "./api.js";
import { confirmSheet, fmtDate, h, icon, openSheet, toast } from "./ui.js";
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
const FIG = { maxFigures: 30, maxSpots: 8, maxPx: 1400, minLabels: 3, bigImage: 0.25, near: 0.04 };
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
  const scale = Math.min(3, FIG.maxPx / r.w);
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
  const r = union([...boxes.filter((b) => chosen.some((l) => near(l, b))), ...chosen]);
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
        line.textContent = `${res.units} ${res.units === 1 ? "Einheit wird" : "Einheiten werden"} vorbereitet${res.figures ? `, ${res.figures} ${res.figures === 1 ? "Grafik" : "Grafiken"} zum Beschriften` : ""}.`;
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

function courseCard(c) {
  const done = c.ready === c.units;
  return h(
    "div.card.course-card",
    h(
      "div.row.between",
      h("h3", c.name, c.mine ? (c.shared ? h("span.badge", "Geteilt") : null) : h("span.badge", `von ${c.owner}`)),
      c.mine ? h("button.btn.ghost", { type: "button", onclick: () => editCourse(c) }, icon("pencil-simple"), "Bearbeiten") : null,
    ),
    c.exam_date ? h("p.muted", `Prüfung am ${fmtDate(c.exam_date)}`) : null,
    h("div.docs", c.documents.map((d) => h("span", icon("file-pdf"), d.name))),
    h("div.bar", h("i", { style: { "--p": String(c.units ? c.ready / c.units : 0) } })),
    h("p.muted.num", done ? `Alle ${c.units} Einheiten bereit` : `${c.ready} von ${c.units} Einheiten vorbereitet`),
  );
}

/** Grafiken eines schon eingelesenen PDFs nachladen: nur Bilder und Beschriftungen, die Einheiten bleiben. */
function reloadFigures(d) {
  const input = h("input", { type: "file", accept: "application/pdf,.pdf" });
  input.addEventListener("change", async () => {
    const file = input.files[0];
    if (!file) return;
    if (file.name !== d.name && !(await confirmSheet({ title: "Andere Datei?", text: `Gewählt ist „${file.name}“, eingelesen wurde „${d.name}“. Die Grafiken werden nach Seitenzahl zugeordnet.`, confirm: "Trotzdem nachladen" }))) return;
    toast("Grafiken werden gesucht …");
    try {
      const { figures } = await readPdf(file, () => {});
      const res = await call("import_figures", { documentId: d.id, figures });
      toast(res.figures ? `${res.figures} ${res.figures === 1 ? "Grafik" : "Grafiken"} übernommen.` : "In dieser Datei wurden keine Grafiken gefunden.");
      emit("course-updated", null);
    } catch (e) {
      toast(String(e?.message || e));
    }
  });
  input.click();
}

function editCourse(c) {
  const name = h("input.input", { id: "edit-name", value: c.name, autocomplete: "off" });
  const date = h("input.input", { id: "edit-date", type: "date", value: c.exam_date || "" });
  // Teilen gibt es nur auf dem eigenen Server: dort sehen alle Konten den Kurs.
  const onServer = state.session?.mode === "server";
  const share = onServer ? h("input.switch", { type: "checkbox", id: "edit-share", checked: c.shared }) : null;
  const save = h("button.btn.block", { type: "button" }, "Speichern");
  const del = h("button.btn.ghost", { type: "button", style: { color: "var(--wrong)" } }, icon("trash"), "Kurs löschen");
  const s = openSheet({
    title: "Kurs bearbeiten",
    body: [
      h("div.field", h("label", { for: "edit-name" }, "Name"), name),
      h("div.field", h("label", { for: "edit-date" }, "Prüfungsdatum"), date),
      c.documents.length
        ? h(
            "div.field",
            h("label", "Grafiken zum Beschriften"),
            h("span.hint", "Für PDFs, die vor dieser Funktion eingelesen wurden: dieselbe Datei noch einmal wählen."),
            ...c.documents.map((d) => h("button.btn.ghost", { type: "button", onclick: () => reloadFigures(d) }, icon("upload-simple"), d.name)),
          )
        : null,
      share
        ? h(
            "div.field",
            h("div.row.between", h("label", { for: "edit-share" }, "Mit allen auf diesem Server teilen"), share),
            h("span.hint", "Alle Konten auf dem Server können dann mit diesem Kurs lernen und Fragen verbessern. Dein Fortschritt bleibt deiner."),
          )
        : null,
      del,
    ],
    foot: save,
  });
  save.addEventListener("click", async () => {
    if (!name.value.trim()) return name.focus();
    await call("update_course", { id: c.id, name: name.value, examDate: date.value || null, shared: share ? share.checked : null });
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
