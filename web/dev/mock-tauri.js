// Nur für die Entwicklung im Browser (ohne Tauri): ahmt den Rust-Kern mit Beispieldaten nach.
// Wird von main.js ausschließlich geladen, wenn window.__TAURI__ fehlt.
// Parameter: ?ai=ready (KI sofort bereit), ?empty (ohne Kurse), ?server (als Server-Konto), ?server&login (Anmeldung)

const q = new URLSearchParams(location.search);
const listeners = {};
const fire = (ev, payload) => (listeners[ev] || []).forEach((fn) => fn({ payload }));
const now = () => Math.floor(Date.now() / 1000);
const DAY = 86400;
const day = (offset) => new Date(Date.now() - offset * DAY * 1000).toLocaleDateString("sv-SE");
const norm = (s) => s.trim().toLowerCase().replace(/[.!?]+$/, "");

const TIERS = [
  { id: "kompakt", label: "Kompakt (Qwen3.5 2B, 1,3 GB)", size: 1280835840 },
  { id: "ausgewogen", label: "Ausgewogen (Qwen3.5 4B, 2,7 GB)", size: 2740937888 },
];
let ai = { state: q.get("ai") === "ready" ? "ready" : "downloading", done: 0, total: TIERS[1].size, tier: "ausgewogen", label: TIERS[1].label, loaded: false };
if (ai.state !== "ready") {
  const t = setInterval(() => {
    ai.done = Math.min(ai.total, ai.done + ai.total / 14);
    if (ai.done >= ai.total) {
      ai = { ...ai, state: "ready" };
      clearInterval(t);
    }
    fire("ai-status", { ...ai });
  }, 450);
}

/** SWOT-Grafik wie aus einer Folie: Feldtitel übermalt, die Stichpunkte bleiben stehen. */
function swotImage() {
  const c = document.createElement("canvas");
  c.width = 900;
  c.height = 600;
  const x = c.getContext("2d");
  x.fillStyle = "#fff";
  x.fillRect(0, 0, 900, 600);
  const fields = [
    ["#d8f1e4", ["Erfahrenes Team", "Treue Stammkunden"]],
    ["#fbdde1", ["Veraltete Kasse", "Wenig Sitzplätze"]],
    ["#dfe6ff", ["Neues Bürogebäude", "Lieferdienste"]],
    ["#fff1bd", ["Kette eröffnet nebenan", "Steigende Mieten"]],
  ];
  fields.forEach(([color, lines], i) => {
    const [fx, fy] = [30 + (i % 2) * 430, 30 + Math.floor(i / 2) * 280];
    x.fillStyle = color;
    x.fillRect(fx, fy, 410, 260);
    x.fillStyle = "#18202f";
    x.font = "22px sans-serif";
    lines.forEach((l, j) => x.fillText(`• ${l}`, fx + 24, fy + 120 + j * 40));
  });
  return c.toDataURL("image/png");
}
const SWOT_SPOTS = [0, 1, 2, 3].map((i) => ({ x: (30 + (i % 2) * 430 + 20) / 900, y: (30 + Math.floor(i / 2) * 280 + 20) / 600, w: 200 / 900, h: 50 / 600, label: ["Stärken", "Schwächen", "Chancen", "Risiken"][i] }));

const firstIdx = (d) => Math.min(...db.units.filter((u) => u.document_id === d.id).map((u) => u.idx));
const db = {
  goal: 20,
  courses: q.has("empty")
    ? []
    : [
        { id: 1, name: "Mikroökonomie", exam_date: day(-23), created_at: 1, owner_id: 1, shared: false },
        { id: 2, name: "Wirtschaftsinformatik", exam_date: day(-40), created_at: 2, owner_id: 1, shared: false },
      ],
  documents: [
    { id: 1, course_id: 1, name: "VL01_Markt_und_Preis.pdf" },
    { id: 2, course_id: 1, name: "VL02_Elastizitaeten.pdf" },
    { id: 3, course_id: 2, name: "WI03_Strategische_Analyse.pdf" },
  ],
  figures: [{ id: 1, unit_id: 8, page: 4, image: swotImage() }],
  units: [
    { id: 1, course_id: 1, document_id: 1, idx: 0, page_from: 1, page_to: 6, title: "Angebot und Nachfrage", summary: "Wie Käufer und Verkäufer auf Preise reagieren und warum Nachfragekurven fallen und Angebotskurven steigen.", status: "ready", q_state: 2 },
    { id: 2, course_id: 1, document_id: 1, idx: 1, page_from: 7, page_to: 12, title: "Das Marktgleichgewicht", summary: "Wo sich Angebot und Nachfrage treffen und was bei Überschuss oder Knappheit passiert.", status: "ready", q_state: 2 },
    { id: 3, course_id: 1, document_id: 1, idx: 2, page_from: 13, page_to: 17, title: "Verschiebungen der Kurven", summary: "Welche Ereignisse eine Kurve verschieben und welche nur eine Bewegung entlang der Kurve auslösen.", status: "ready", q_state: 2 },
    { id: 4, course_id: 1, document_id: 2, idx: 3, page_from: 1, page_to: 5, title: "Preiselastizität der Nachfrage", summary: "Wie stark die nachgefragte Menge reagiert, wenn sich der Preis ändert.", status: "ready", q_state: 2 },
    { id: 5, course_id: 1, document_id: 2, idx: 4, page_from: 6, page_to: 10, title: "Kreuzpreis- und Einkommenselastizität", summary: "Substitute, Komplemente und was ein höheres Einkommen mit der Nachfrage macht.", status: "ready", q_state: 1 },
    { id: 6, course_id: 1, document_id: 2, idx: 5, page_from: 11, page_to: 15, title: "Seiten 11-15", summary: "", status: "pending", q_state: 0 },
    { id: 7, course_id: 1, document_id: 2, idx: 6, page_from: 16, page_to: 19, title: "Seiten 16-19", summary: "", status: "pending", q_state: 0 },
    { id: 8, course_id: 2, document_id: 3, idx: 0, page_from: 1, page_to: 8, title: "SWOT-Analyse", summary: "Wie man interne Stärken und Schwächen mit externen Chancen und Risiken verbindet und daraus Strategien ableitet.", status: "ready", q_state: 2 },
  ],
  terms: [
    [1, "Nachfragekurve", "Zeigt, welche Menge Käufer bei jedem Preis kaufen möchten."],
    [1, "Angebotskurve", "Zeigt, welche Menge Verkäufer bei jedem Preis anbieten."],
    [1, "Ceteris paribus", "Alle anderen Einflüsse bleiben gleich, nur eine Größe ändert sich."],
    [1, "Gesetz der Nachfrage", "Steigt der Preis, sinkt die nachgefragte Menge."],
    [2, "Gleichgewichtspreis", "Der Preis, bei dem angebotene und nachgefragte Menge gleich sind."],
    [2, "Angebotsüberschuss", "Beim aktuellen Preis wird mehr angeboten als nachgefragt."],
    [2, "Nachfrageüberschuss", "Beim aktuellen Preis wird mehr nachgefragt als angeboten."],
    [3, "Komplementärgut", "Ein Gut, das zusammen mit einem anderen genutzt wird, etwa Drucker und Tinte."],
    [3, "Substitut", "Ein Gut, das ein anderes ersetzen kann, etwa Butter und Margarine."],
    [3, "Bewegung entlang der Kurve", "Folge einer Preisänderung des Gutes selbst."],
    [4, "Preiselastizität", "Prozentuale Mengenänderung geteilt durch prozentuale Preisänderung."],
    [4, "Unelastisch", "Die Menge reagiert kaum auf den Preis, der Betrag der Elastizität ist kleiner als 1."],
    [4, "Elastisch", "Die Menge reagiert stark auf den Preis, der Betrag ist größer als 1."],
    [8, "SWOT-Analyse", "Stellt Stärken und Schwächen des Unternehmens den Chancen und Risiken am Markt gegenüber."],
    [8, "Interne Analyse", "Betrachtet, was das Unternehmen selbst beeinflussen kann: Stärken und Schwächen."],
    [8, "Externe Analyse", "Betrachtet Markt und Umwelt: Chancen und Risiken."],
  ].map(([unit_id, term, definition], i) => ({ id: i + 1, unit_id, term, definition })),
  questions: [
    [1, "mc", "Was beschreibt das Gesetz der Nachfrage?", ["Steigt der Preis, sinkt die nachgefragte Menge", "Steigt der Preis, steigt die nachgefragte Menge", "Der Preis hat keinen Einfluss auf die Menge", "Das Angebot bestimmt allein die Nachfrage"], "Steigt der Preis, sinkt die nachgefragte Menge", "Höhere Preise machen ein Gut für viele Käufer weniger attraktiv."],
    [1, "true_false", "Die Angebotskurve verläuft normalerweise steigend.", ["Richtig", "Falsch"], "Richtig", "Bei höheren Preisen lohnt es sich für Verkäufer, mehr anzubieten."],
    [1, "fill_blank", "„Alle anderen Einflüsse bleiben gleich“ heißt in der Ökonomie ___.", ["ceteris paribus", "laissez faire", "pro rata", "de facto"], "ceteris paribus", "Ceteris paribus isoliert die Wirkung einer einzigen Größe."],
    [1, "mc", "Welche Größe steht bei der Nachfragekurve auf der senkrechten Achse?", ["Der Preis", "Die Menge", "Das Einkommen", "Die Kosten"], "Der Preis", "In der Ökonomie trägt man den Preis traditionell senkrecht ab."],
    [1, "true_false", "Eine Preisänderung verschiebt die Nachfragekurve.", ["Richtig", "Falsch"], "Falsch", "Eine Preisänderung führt zu einer Bewegung entlang der Kurve, nicht zu einer Verschiebung."],
    [2, "mc", "Was passiert bei einem Preis über dem Gleichgewichtspreis?", ["Es entsteht ein Angebotsüberschuss", "Es entsteht ein Nachfrageüberschuss", "Der Markt ist im Gleichgewicht", "Die Nachfragekurve verschiebt sich"], "Es entsteht ein Angebotsüberschuss", "Bei hohem Preis bieten Verkäufer mehr an, als Käufer haben wollen."],
    [2, "fill_blank", "Im Gleichgewicht sind angebotene und nachgefragte ___ gleich.", ["Menge", "Kosten", "Elastizität", "Nutzen"], "Menge", "Der Gleichgewichtspreis gleicht die Mengen aus."],
    [2, "true_false", "Bei einem Nachfrageüberschuss steigt der Preis tendenziell.", ["Richtig", "Falsch"], "Richtig", "Knappheit lässt Käufer mehr bieten."],
    [3, "mc", "Der Preis für Tinte steigt stark. Was passiert mit der Nachfrage nach Druckern?", ["Sie sinkt", "Sie steigt", "Sie bleibt gleich", "Sie wird unendlich elastisch"], "Sie sinkt", "Drucker und Tinte sind Komplemente."],
    [4, "mc", "Die Elastizität beträgt -0,4. Wie reagiert die Nachfrage?", ["Unelastisch", "Elastisch", "Vollkommen elastisch", "Gar nicht"], "Unelastisch", "Der Betrag ist kleiner als 1."],
    [8, "sort", "Ordne die Aussagen den vier Feldern der SWOT-Analyse zu.", [], "", "Stärken und Schwächen liegen im Unternehmen, Chancen und Risiken kommen von außen.", {
      groups: [
        { name: "Stärken", items: ["Gut ausgebildetes Personal", "Bekannte Marke"] },
        { name: "Schwächen", items: ["Veraltete IT", "Hohe Kosten"] },
        { name: "Chancen", items: ["Wachsender Online-Markt", "Neue Zielgruppe"] },
        { name: "Risiken", items: ["Neue Konkurrenz", "Strengeres Gesetz"] },
      ],
    }],
    [8, "sort", "Das Café „Bohne“ liegt in der Innenstadt. Erstelle seine SWOT-Analyse: Was gehört wohin?", ["Der Kaffee schmeckt nach Kaffee", "Im Winter ist es kalt"], "", "Nicht jede Aussage über das Café gehört in die Analyse.", {
      groups: [
        { name: "Stärken", items: ["Barista mit 10 Jahren Erfahrung"] },
        { name: "Schwächen", items: ["Nur 12 Sitzplätze"] },
        { name: "Chancen", items: ["Neues Bürogebäude gegenüber"] },
        { name: "Risiken", items: ["Eine Kette eröffnet nebenan"] },
      ],
    }],
    [8, "order", "Bring die Schritte einer SWOT-Analyse in die richtige Reihenfolge.", ["Ziel der Analyse festlegen", "Stärken und Schwächen erfassen", "Chancen und Risiken erfassen", "Felder in der Matrix kombinieren", "Strategien ableiten"], "", "Erst intern, dann extern, dann kombinieren."],
    [8, "cloze", "Die SWOT-Analyse verbindet eine [interne] Analyse der Stärken und Schwächen mit einer [externen] Analyse der Chancen und Risiken.", ["finanzielle", "politische"], "", "Intern = im Unternehmen, extern = Markt und Umwelt."],
    [8, "label", "Beschrifte die Felder der SWOT-Matrix.", [], "", "Oben steht, was hilft und schadet; links, ob es von innen oder außen kommt.", { spots: SWOT_SPOTS, figure_id: 1 }],
  ].map(([unit_id, kind, prompt, options, answer, why, extra = {}], i) => summarize({ id: i + 1, unit_id, kind, prompt, options, answer, why, groups: [], spots: [], figure_id: null, ...extra, box: i % 4 === 1 && unit_id !== 8 ? 2 : 0, due_at: i % 4 === 1 && unit_id !== 8 ? now() - 10 : 0, hidden: false })),
  lessons: q.has("empty")
    ? []
    : [
        { unit_id: 1, xp: 22, at: now() - 1 * DAY },
        { unit_id: 2, xp: 17, at: now() - 2 * DAY },
        { unit_id: 1, xp: 25, at: now() - 4 * DAY },
      ],
  nextId: 100,
};
if (q.has("empty")) db.units = [];
const visible = () => db.questions.filter((x) => !x.hidden);
let user = q.has("server") && !q.has("login") ? { name: "Alex", admin: true } : null;
const sessions = [
  { id: 1, label: "Firefox · Linux", ip: "192.168.1.20", last_seen: now() - 60, current: true },
  { id: 2, label: "Lernpfad-App · Android", ip: "192.168.1.31", last_seen: now() - 3 * DAY, current: false },
];
const members = [
  { id: 1, name: "Alex", admin: true, disabled: false, me: true },
  { id: 2, name: "Sam", admin: false, disabled: false, me: false },
  { id: 3, name: "Jo", admin: false, disabled: false, me: false },
  { id: 4, name: "Mira", admin: false, disabled: false, me: false },
  { id: 5, name: "Kemal", admin: false, disabled: false, me: false },
];
const courseMembers = new Map(); // Kurs → Set der ausgewählten Personen

function streak() {
  const days = [...new Set(db.lessons.map((l) => Math.floor((l.at - new Date().getTimezoneOffset() * 60) / DAY)))].sort((a, b) => b - a);
  const today = Math.floor((now() - new Date().getTimezoneOffset() * 60) / DAY);
  if (!days.length || days[0] < today - 1) return 0;
  let n = 1;
  for (let i = 1; i < days.length && days[i - 1] - days[i] === 1; i++) n++;
  return n;
}
function stats() {
  const xpOn = (d) => db.lessons.filter((l) => new Date(l.at * 1000).toLocaleDateString("sv-SE") === d).reduce((a, l) => a + l.xp, 0);
  const week = [6, 5, 4, 3, 2, 1, 0].map((o) => ({ day: day(o), xp: xpOn(day(o)) }));
  return {
    streak: streak(),
    xp_today: week[6].xp,
    goal: db.goal,
    total_xp: db.lessons.reduce((a, l) => a + l.xp, 0),
    lessons: db.lessons.length,
    due: visible().filter((x) => x.box > 0 && x.due_at <= now()).length,
    week,
    active_days: [...new Set(db.lessons.map((l) => new Date(l.at * 1000).toLocaleDateString("sv-SE")))],
  };
}
const shuffle = (a) => [...a].sort(() => Math.random() - 0.5);

// ---------- Strukturaufgaben wie im Kern (core/src/learn.rs, api.rs) ----------
function clozeParts(prompt) {
  const gaps = [];
  const text = prompt.replace(/\[([^\]]*)\]/g, (_, g) => (gaps.push(g.trim()), "___"));
  return [text, gaps];
}
function summarize(x) {
  if (x.kind === "sort") x.answer = x.groups.map((g) => `${g.name}: ${g.items.join(", ")}`).join(" · ");
  if (x.kind === "order") x.answer = x.options.join(" → ");
  if (x.kind === "cloze") x.answer = clozeParts(x.prompt)[1].join(" · ");
  if (x.kind === "label") x.answer = x.spots.map((s, i) => `${i + 1} ${s.label}`).join(" · ");
  return x;
}
function toExercise(x) {
  let { prompt, options } = x;
  if (x.kind === "sort") options = [...x.groups.flatMap((g) => g.items), ...options];
  if (x.kind === "cloze") [prompt, options] = [clozeParts(prompt)[0], [...clozeParts(prompt)[1], ...options]];
  if (x.kind === "label") options = [...x.spots.map((s) => s.label), ...options];
  if (x.kind !== "true_false") {
    const solved = options.join("|");
    do options = shuffle(options);
    while (x.kind === "order" && options.join("|") === solved);
  }
  return {
    id: x.id,
    kind: x.kind,
    prompt,
    options,
    pairs: [],
    groups: x.groups.map((g) => g.name),
    spots: x.spots.map((s) => [s.x, s.y, s.w, s.h]),
    image: x.kind === "label" ? db.figures.find((f) => f.id === x.figure_id)?.image ?? null : null,
  };
}
const passed = (m, n) => m === 0 || (m === 1 && n >= 6);
function grade(x, given) {
  let parsed;
  try {
    parsed = JSON.parse(given);
  } catch {
    parsed = null;
  }
  if (x.kind === "sort") {
    const placed = new Map(Object.entries(parsed ?? {}).flatMap(([g, items]) => items.map((i) => [norm(i), norm(g)])));
    const wrong = x.groups.flatMap((g) => g.items.filter((i) => placed.get(norm(i)) !== norm(g.name))).length + x.options.filter((o) => placed.has(norm(o))).length;
    return { correct: passed(wrong, x.groups.flatMap((g) => g.items).length + x.options.length), solution: x.groups };
  }
  const want = x.kind === "order" ? x.options : x.kind === "cloze" ? clozeParts(x.prompt)[1] : x.spots.map((s) => s.label);
  const got = Array.isArray(parsed) ? parsed : [];
  return { correct: passed(want.filter((w, i) => norm(got[i] ?? "") !== norm(w)).length, want.length), solution: want };
}
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function streamText(text, ch) {
  let out = "";
  for (const w of text.split(/(\s+)/)) {
    out += w;
    ch.onmessage?.(w);
    await wait(18);
  }
  return out;
}

const EXPLAIN = `## Worum geht's?
Die **Nachfrage** beschreibt, wie viel Käufer von einem Gut haben wollen. Sie hängt vor allem vom **Preis** ab.

## So funktioniert es
Steigt der Preis, kaufen viele Menschen weniger. Das ist das **Gesetz der Nachfrage**.

Die **Angebotskurve** zeigt das Gegenstück: Bei höheren Preisen bieten Verkäufer mehr an.

## Typische Denkfehler
- Eine Preisänderung **verschiebt** die Kurve nicht, sie führt zu einer Bewegung entlang der Kurve.
- „Nachfrage“ und „nachgefragte Menge“ sind nicht dasselbe.

## Beispiel
Wird Kaffee teurer, trinken manche lieber Tee. Die nachgefragte Menge Kaffee sinkt.

> **Merke:** Preis hoch, nachgefragte Menge runter.`;

const handlers = {
  ai_status: () => ({ ...ai }),
  ai_tiers: () => TIERS,
  ai_retry: () => null,
  ai_set_tier: ({ tier }) => {
    const t = TIERS.find((x) => x.id === tier);
    ai = { ...ai, tier, label: t.label, total: t.size };
    fire("ai-status", { ...ai });
  },
  courses: () =>
    db.courses.map((c) => {
      const us = db.units.filter((u) => u.course_id === c.id);
      const picked = courseMembers.get(c.id)?.size ?? 0;
      const wrong = visible().filter((x) => x.wrong && us.some((u) => u.id === x.unit_id)).length;
      return {
        ...c,
        mine: true,
        owner: user?.name || "Ich",
        shared: !!c.shared || picked > 0,
        share_all: !!c.shared,
        members: picked,
        members_invite: !!c.members_invite,
        can_invite: true,
        wrong,
        units: us.length,
        ready: us.filter((u) => u.status === "ready").length,
        documents: db.documents
          .filter((d) => d.course_id === c.id)
          .sort((a, b) => firstIdx(a) - firstIdx(b))
          .map((d) => ({ id: d.id, name: d.name })),
      };
    }),
  create_course: ({ name, examDate }) => {
    const id = db.nextId++;
    db.courses.push({ id, name, exam_date: examDate, created_at: now(), owner_id: 1, shared: false });
    return id;
  },
  update_course: ({ id, name, examDate, share, members: picked, membersInvite }) => {
    const c = db.courses.find((x) => x.id === id);
    Object.assign(c, { name, exam_date: examDate }, share ? { shared: share === "all" } : {}, membersInvite == null ? {} : { members_invite: membersInvite });
    if (share === "none") courseMembers.delete(id);
    if (share === "some" && picked) courseMembers.set(id, new Set(picked));
  },
  share_people: ({ courseId }) => members.filter((m) => !m.me).map((m) => ({ id: m.id, name: m.name, member: !!courseMembers.get(courseId)?.has(m.id) })),
  invite_to_course: ({ courseId, userIds }) => courseMembers.set(courseId, new Set([...(courseMembers.get(courseId) ?? []), ...userIds])),
  delete_course: ({ id }) => {
    db.courses = db.courses.filter((c) => c.id !== id);
    db.units = db.units.filter((u) => u.course_id !== id);
  },
  import_document: ({ courseId, name, pages, figures = [] }) => {
    const text = pages.filter((p) => p.trim());
    if (!text.length) throw "In diesem PDF wurde kein Text gefunden. Ist es eingescannt?";
    const docId = db.nextId++;
    db.documents.push({ id: docId, course_id: courseId, name });
    const n = Math.max(1, Math.round(text.join("").length / 5000));
    let idx = db.units.filter((u) => u.course_id === courseId).length;
    for (let i = 0; i < n; i++) db.units.push({ id: db.nextId++, course_id: courseId, document_id: docId, idx: idx++, page_from: i * 5 + 1, page_to: i * 5 + 5, title: `Seiten ${i * 5 + 1}-${i * 5 + 5}`, summary: "", status: "pending", q_state: 0 });
    // Im Mock werden neue Einheiten gleich „fertig“, damit man die Grafik-Aufgaben direkt ausprobieren kann.
    db.units.filter((u) => u.document_id === docId).forEach((u) => Object.assign(u, { status: "ready", title: u.title.replace("Seiten", "Abschnitt, Seiten") }));
    return { units: n, figures: storeFigures(docId, figures), skipped: 0 };
  },
  // Im Mock einfach: alle Einheiten des PDFs neu, als „geändert“ gezählt
  replace_document: ({ documentId, name, pages, figures = [] }) => {
    if (!pages.some((p) => p.trim())) throw "In diesem PDF wurde kein Text gefunden. Ist es eingescannt?";
    const d = db.documents.find((x) => x.id === documentId);
    d.name = name;
    db.figures = db.figures.filter((f) => db.units.find((u) => u.id === f.unit_id)?.document_id !== documentId || f.page == null);
    const changed = db.units.filter((u) => u.document_id === documentId).length;
    return { units: 0, same: 0, changed, removed: 0, figures: storeFigures(documentId, figures), skipped: 0 };
  },
  reorder_documents: ({ courseId, documentIds }) => {
    const rank = (u) => (documentIds.includes(u.document_id) ? documentIds.indexOf(u.document_id) : 1e9);
    db.units
      .filter((u) => u.course_id === courseId)
      .sort((a, b) => rank(a) - rank(b) || a.idx - b.idx)
      .forEach((u, i) => (u.idx = i));
  },
  delete_document: ({ documentId }) => {
    const gone = new Set(db.units.filter((u) => u.document_id === documentId).map((u) => u.id));
    db.questions = db.questions.filter((x) => !gone.has(x.unit_id));
    db.units = db.units.filter((u) => !gone.has(u.id));
    db.documents = db.documents.filter((d) => d.id !== documentId);
  },
  figure: ({ figureId }) => db.figures.find((f) => f.id === figureId) ?? null,
  unit_figures: ({ unitId }) => db.figures.filter((f) => f.unit_id === unitId),
  course_path: ({ courseId }) =>
    db.units
      .filter((u) => u.course_id === courseId)
      .sort((a, b) => a.idx - b.idx)
      .map((u) => {
        const qs = visible().filter((x) => x.unit_id === u.id);
        return {
          ...u,
          document: db.documents.find((d) => d.id === u.document_id).name,
          questions: qs.length,
          mastery: qs.length ? qs.reduce((a, x) => a + x.box, 0) / qs.length : 0,
          due: qs.filter((x) => x.box > 0 && x.due_at <= now()).length,
          lessons: db.lessons.filter((l) => l.unit_id === u.id).length,
        };
      }),
  unit: ({ unitId }) => {
    const u = db.units.find((x) => x.id === unitId);
    return { ...u, document: db.documents.find((d) => d.id === u.document_id).name, explanation: u.explanation ?? null, terms: db.terms.filter((t) => t.unit_id === unitId).map((t) => ({ ...t, unit_title: u.title })) };
  },
  search_terms: ({ query }) => db.terms.filter((t) => (t.term + t.definition).toLowerCase().includes(query.toLowerCase())).map((t) => ({ ...t, unit_title: db.units.find((u) => u.id === t.unit_id).title })),
  explain: async ({ unitId, onToken }) => {
    const u = db.units.find((x) => x.id === unitId);
    if (u.explanation) return u.explanation;
    u.explanation = await streamText(EXPLAIN, onToken);
    return u.explanation;
  },
  ask: ({ onToken }) => streamText("Gute Frage! Bei einem **höheren Preis** lohnt sich der Kauf für weniger Menschen. Deshalb sinkt die nachgefragte Menge, solange alles andere gleich bleibt.", onToken),
  start_lesson: ({ unitId, wrongCourse }) => {
    const inCourse = (x) => db.units.find((u) => u.id === x.unit_id)?.course_id === wrongCourse;
    const pool = wrongCourse != null ? visible().filter((x) => x.wrong && inCourse(x)) : unitId == null ? visible().filter((x) => x.box > 0 && x.due_at <= now()) : visible().filter((x) => x.unit_id === unitId);
    if (wrongCourse != null) return { ready: true, failed: false, exercises: shuffle(pool).slice(0, 12).map(toExercise) };
    if (!pool.length && unitId != null) return { ready: false, failed: false, exercises: [] };
    const ex = shuffle(pool).slice(0, 8).map(toExercise);
    const terms = shuffle(db.terms.filter((t) => (unitId == null ? pool.some((p) => p.unit_id === t.unit_id) : t.unit_id === unitId))).slice(0, 4);
    if (terms.length >= 3) ex.splice(Math.min(3, ex.length), 0, { id: null, kind: "match", prompt: "Ordne die Begriffe zu", options: [], pairs: terms.map((t) => [t.term, t.definition]), groups: [], spots: [], image: null });
    return { ready: true, failed: false, exercises: ex };
  },
  answer: ({ questionId, given, retry }) => {
    const x = db.questions.find((y) => y.id === questionId);
    const { correct, solution = null } = ["sort", "order", "cloze", "label"].includes(x.kind) ? grade(x, given) : { correct: norm(given) === norm(x.answer) };
    if (!retry) {
      x.wrong = !correct;
      x.box = correct ? Math.min(5, x.box + 1) : 1;
      x.due_at = now() + [0, 1, 3, 7, 14, 30][x.box] * DAY;
    }
    return { correct, expected: x.answer, why: x.why, solution };
  },
  stats: () => stats(),
  finish_lesson: ({ unitId, correct, total }) => {
    const xp = 2 * correct + 5 + (total > 0 && correct >= total ? 5 : 0);
    db.lessons.push({ unit_id: unitId, xp, at: now() });
    return { xp, stats: stats() };
  },
  set_goal: ({ xp }) => (db.goal = xp),
  retry_questions: () => null,
  unit_questions: ({ unitId }) => db.questions.filter((x) => x.unit_id === unitId).map(full),
  question: ({ questionId }) => full(db.questions.find((x) => x.id === questionId)),
  create_question: ({ unitId, kind, ...rest }) => {
    const x = { id: db.nextId++, unit_id: unitId, kind, prompt: "", options: [], answer: "", why: "", groups: [], spots: [], figure_id: null, box: 0, due_at: 0, hidden: false };
    db.questions.push(x);
    try {
      return handlers.update_question({ questionId: x.id, ...rest });
    } catch (e) {
      db.questions = db.questions.filter((y) => y !== x);
      throw e;
    }
  },
  update_question: ({ questionId, prompt, options = [], answer = "", why = "", groups = [], spots = [], figureId, image }) => {
    const x = db.questions.find((y) => y.id === questionId);
    if (prompt.trim().length < 8) throw "So geht die Aufgabe nicht: Sie braucht mindestens 8 Zeichen.";
    if (["sort", "order", "cloze", "label"].includes(x.kind)) {
      if (x.kind === "sort" && groups.filter((g) => g.items.length).length < 2) throw "So geht die Aufgabe nicht: Zuordnen braucht 2 bis 4 Felder.";
      if (x.kind === "order" && options.length < 3) throw "So geht die Aufgabe nicht: Eine Reihenfolge braucht 3 bis 7 Schritte.";
      if (x.kind === "cloze" && !clozeParts(prompt)[1].length) throw "So geht die Aufgabe nicht: Markiere 1 bis 5 Lücken mit [eckigen Klammern].";
      if (x.kind === "label") {
        if (!spots.length || spots.some((s) => !s.label)) throw "So geht die Aufgabe nicht: Setze Stellen ins Bild und beschrifte jede.";
        if (image) db.figures.push({ id: (figureId = db.nextId++), unit_id: x.unit_id, page: null, image });
        x.figure_id = figureId ?? x.figure_id;
      }
      Object.assign(x, { prompt: prompt.trim(), options, why: why.trim(), groups: groups.filter((g) => g.items.length), spots });
      return full(summarize(x));
    }
    if (x.kind === "true_false") options = ["Richtig", "Falsch"];
    if (!options.some((o) => norm(o) === norm(answer))) throw "So geht die Frage nicht: Die richtige Antwort muss unter den Optionen stehen.";
    Object.assign(x, { prompt: prompt.trim(), options, answer: options.find((o) => norm(o) === norm(answer)), why: why.trim() });
    return full(x);
  },
  hide_question: ({ questionId, hidden }) => (db.questions.find((x) => x.id === questionId).hidden = hidden),
  delete_question: ({ questionId }) => (db.questions = db.questions.filter((x) => x.id !== questionId)),
  // Server-Modus (?server): Konto, Geräte, Mitglieder
  session: () => (q.has("server") ? { mode: "server", user, setup: false } : { mode: "local", user: null }),
  login: ({ name, password }) => {
    if (password !== "richtig-gutes-passwort") {
      if (++fails >= 4) throw { locked: 30, message: "Zu viele Versuche." };
      throw "Name oder Passwort stimmt nicht.";
    }
    user = { name, admin: true };
    return { ok: true };
  },
  logout: () => (user = null),
  sessions_list: () => sessions,
  session_revoke: ({ id }) => sessions.splice(sessions.findIndex((x) => x.id === id), 1),
  sessions_revoke_others: () => sessions.splice(0, sessions.length, ...sessions.filter((x) => x.current)),
  change_password: ({ current }) => {
    if (current !== "richtig-gutes-passwort") throw "Das aktuelle Passwort stimmt nicht.";
  },
  members: () => members,
  member_disable: ({ id, disabled }) => (members.find((m) => m.id === id).disabled = disabled),
  invite_create: ({ userId }) => ({ link: `${location.origin}/#/${userId ? "reset" : "join"}/c0ffee${Date.now().toString(16)}`, expires_at: now() + 7 * DAY }),
  connect_link: () => ({ link: `https://${location.host}/#pin=${"ab".repeat(32)}` }),
};
let fails = 0;
function full(x) {
  return { id: x.id, unit_id: x.unit_id, kind: x.kind, prompt: x.prompt, options: x.options, answer: x.answer, why: x.why, groups: x.groups, spots: x.spots, figure_id: x.figure_id, hidden: x.hidden, mine: true, shared: !!db.courses[0]?.shared };
}
function storeFigures(documentId, figures) {
  let n = 0;
  for (const f of figures.slice(0, 30)) {
    const units = db.units.filter((u) => u.document_id === documentId);
    const u = units.find((x) => f.page >= x.page_from && f.page <= x.page_to) ?? units[0];
    if (!u) continue;
    const id = db.nextId++;
    db.figures.push({ id, unit_id: u.id, page: f.page, image: f.image });
    n++;
    if (f.spots.length >= 3) db.questions.push(summarize({ id: db.nextId++, unit_id: u.id, kind: "label", prompt: f.title ? `Beschrifte die Grafik „${f.title}“.` : "Beschrifte die Grafik.", options: [], answer: "", why: "", groups: [], spots: f.spots, figure_id: id, box: 0, due_at: 0, hidden: false }));
  }
  return n;
}

class Channel {
  onmessage = null;
}

window.__TAURI__ = {
  core: {
    Channel,
    // wie src-tauri/src/lib.rs: rpc(cmd, args) und rpc_stream(cmd, args, onToken)
    async invoke(_name, { cmd, args = {}, onToken } = {}) {
      await wait(60);
      if (!handlers[cmd]) throw `Unbekannter Befehl: ${cmd}`;
      return structuredClone((await handlers[cmd]({ ...args, onToken })) ?? null);
    },
  },
  event: {
    async listen(ev, fn) {
      (listeners[ev] ||= []).push(fn);
      return () => (listeners[ev] = listeners[ev].filter((f) => f !== fn));
    },
  },
};
