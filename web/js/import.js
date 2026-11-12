// Hinzufügen: Kurs anlegen, PDF einlesen (pdf.js, läuft in jeder WebView), Fortschritt der Vorbereitung.
import { call } from "./api.js";
import { confirmSheet, fmtDate, h, icon, openSheet, toast } from "./ui.js";
import { aiNotice, currentCourse, emit, loadCourses, selectCourse, state, subscribe } from "./state.js";

let pdfjs;
async function readPdf(file, onPage) {
  pdfjs ??= await import("../vendor/pdf.min.mjs").then((m) => {
    m.GlobalWorkerOptions.workerSrc = new URL("../vendor/pdf.worker.min.mjs", import.meta.url).href;
    return m;
  });
  const task = pdfjs.getDocument({ data: new Uint8Array(await file.arrayBuffer()), isEvalSupported: false });
  const doc = await task.promise;
  const pages = [];
  for (let n = 1; n <= doc.numPages; n++) {
    const page = await doc.getPage(n);
    const content = await page.getTextContent();
    pages.push(content.items.map((it) => (it.str ?? "") + (it.hasEOL ? "\n" : " ")).join(""));
    onPage(n, doc.numPages);
  }
  await task.destroy();
  return pages;
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
        const pages = await readPdf(file, (n, of) => {
          line.textContent = `Seite ${n} von ${of} gelesen`;
          bar.style.setProperty("--p", String(n / of));
        });
        const res = await call("import_document", { courseId, name: file.name, pages });
        line.textContent = `${res.units} ${res.units === 1 ? "Einheit wird" : "Einheiten werden"} vorbereitet.`;
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
    h("div.docs", c.documents.map((d) => h("span", icon("file-pdf"), d))),
    h("div.bar", h("i", { style: { "--p": String(c.units ? c.ready / c.units : 0) } })),
    h("p.muted.num", done ? `Alle ${c.units} Einheiten bereit` : `${c.ready} von ${c.units} Einheiten vorbereitet`),
  );
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
