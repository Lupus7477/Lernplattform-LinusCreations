// Lernen: der Lernpfad eines Kurses.
import { call } from "./api.js";
import { daysUntil, docName, fmtDate, h, icon, openSheet } from "./ui.js";
import { aiNotice, currentCourse, subscribe } from "./state.js";

export async function render() {
  const page = h("div.page");
  const draw = async () => page.replaceChildren(...(await content()));
  await draw();
  page.cleanup = subscribe((kind) => {
    if (["ai", "course-updated", "course-selected", "courses"].includes(kind)) draw();
  });
  return page;
}

async function content() {
  const course = currentCourse();
  if (!course) {
    return [
      aiNotice(),
      h(
        "div.empty",
        h("div.art", icon("book-open-text")),
        h("h1", "Lade dein erstes Skript"),
        h("p", "Aus deinem PDF entstehen kurze Lektionen mit Fragen. Alles läuft auf diesem Gerät, ohne Internet."),
        h("a.btn", { href: "#/add" }, icon("upload-simple"), "Skript hinzufügen"),
      ),
    ].filter(Boolean);
  }

  const units = await call("course_path", { courseId: course.id });
  const pending = units.filter((u) => u.status !== "ready").length;
  const open = units.filter((u) => u.lessons === 0).length;

  const head = h("div.course-head", h("h1", course.name));
  if (course.exam_date) {
    const days = daysUntil(course.exam_date);
    const perDay = days > 0 ? Math.ceil(open / days) : open;
    const plan = days < 0 ? "Die Prüfung liegt hinter dir." : days === 0 ? "Heute ist Prüfung. Viel Erfolg!" : perDay > 0 ? `Schaffe heute ${perDay} ${perDay === 1 ? "Lektion" : "Lektionen"}, dann bist du rechtzeitig durch.` : "Alle Einheiten einmal geübt. Jetzt wiederholen.";
    const when = days > 1 ? `, noch ${days} Tage` : days === 1 ? ", also morgen" : "";
    head.append(h("p.exam", icon("calendar-blank"), `Prüfung am ${fmtDate(course.exam_date)}${when}`), h("p.muted", plan));
  }
  if (pending > 0) {
    head.append(h("p.muted", { role: "status" }, `Die KI bereitet noch ${pending} von ${units.length} Einheiten vor. Du kannst schon loslegen.`));
  }

  return [aiNotice(), head, pathOf(units)].filter(Boolean);
}

function pathOf(units) {
  const path = h("div.path");
  const next = units.find((u) => u.status === "ready" && u.lessons === 0);
  let doc = null;
  let i = 0;
  for (const u of units) {
    if (u.document_id !== doc) {
      doc = u.document_id;
      i = 0;
      const count = units.filter((x) => x.document_id === doc).length;
      path.append(h("div.doc-banner", h("b", docName(u.document)), h("span", `${count} ${count === 1 ? "Einheit" : "Einheiten"}`)));
    }
    path.append(node(u, u === next, i++));
  }
  return path;
}

function node(u, isNext, i) {
  const ready = u.status === "ready";
  const mastered = ready && u.questions > 0 && u.mastery >= 4;
  const kind = !ready ? "pending" : mastered ? "mastered" : u.lessons > 0 ? "done" : "";
  const glyph = !ready ? "hourglass-medium" : mastered ? "crown-fill" : u.lessons > 0 ? "check" : isNext ? "star-fill" : "book-open-text";
  const sub = !ready ? "Wird vorbereitet" : u.q_state === 1 && u.questions === 0 ? "Fragen entstehen" : u.due > 0 ? `${u.due} fällig` : null;
  // Sanfte Schlangenlinie, damit der Pfad wie ein Weg wirkt
  const x = `${Math.round(Math.sin(i * 0.9) * 62)}px`;
  return h(
    "div.node-wrap",
    { style: { "--x": x } },
    isNext ? h("span.node-start", { "aria-hidden": "true" }, "Start") : null,
    h(
      `button.node${kind ? "." + kind : ""}`,
      { type: "button", disabled: !ready, "aria-label": `${u.title}${sub ? ", " + sub : ""}`, onclick: () => unitSheet(u) },
      ready && u.questions > 0 ? h("span.node-ring", { style: { "--m": String(Math.min(1, u.mastery / 5)) } }) : null,
      icon(glyph),
    ),
    h("span.node-label", u.title),
    sub ? h("span.node-sub", sub) : null,
  );
}

function unitSheet(u) {
  const info =
    u.questions > 0
      ? `${u.questions} Fragen${u.due ? `, ${u.due} davon fällig` : ""}. ${u.lessons ? `${u.lessons}× geübt.` : "Noch nicht geübt."}`
      : u.q_state === 3
        ? "Die Fragen konnten nicht erstellt werden. Starte die Lektion, um es erneut zu versuchen."
        : "Die Fragen werden gerade erstellt. Das dauert meist unter einer Minute.";
  const s = openSheet({
    title: u.title,
    body: [u.summary ? h("p", u.summary) : null, h("p.muted", info)],
    foot: [
      h("a.btn.block", { href: `#/lesson/${u.id}`, onclick: () => s.close() }, u.lessons ? "Nochmal üben" : "Lektion starten"),
      h("a.btn.secondary.block", { href: `#/unit/${u.id}`, onclick: () => s.close() }, icon("book-open-text"), "Lernkarte ansehen"),
    ],
  });
}
