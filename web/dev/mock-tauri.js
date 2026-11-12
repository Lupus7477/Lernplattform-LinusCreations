// Nur für die Entwicklung im Browser (ohne Tauri): ahmt den Rust-Kern mit Beispieldaten nach.
// Wird von main.js ausschließlich geladen, wenn window.__TAURI__ fehlt.
// Parameter: ?ai=ready (KI sofort bereit), ?empty (ohne Kurse)

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

const db = {
  goal: 20,
  courses: q.has("empty") ? [] : [{ id: 1, name: "Mikroökonomie", exam_date: day(-23), created_at: 1 }],
  documents: [
    { id: 1, course_id: 1, name: "VL01_Markt_und_Preis.pdf" },
    { id: 2, course_id: 1, name: "VL02_Elastizitaeten.pdf" },
  ],
  units: [
    { id: 1, course_id: 1, document_id: 1, idx: 0, page_from: 1, page_to: 6, title: "Angebot und Nachfrage", summary: "Wie Käufer und Verkäufer auf Preise reagieren und warum Nachfragekurven fallen und Angebotskurven steigen.", status: "ready", q_state: 2 },
    { id: 2, course_id: 1, document_id: 1, idx: 1, page_from: 7, page_to: 12, title: "Das Marktgleichgewicht", summary: "Wo sich Angebot und Nachfrage treffen und was bei Überschuss oder Knappheit passiert.", status: "ready", q_state: 2 },
    { id: 3, course_id: 1, document_id: 1, idx: 2, page_from: 13, page_to: 17, title: "Verschiebungen der Kurven", summary: "Welche Ereignisse eine Kurve verschieben und welche nur eine Bewegung entlang der Kurve auslösen.", status: "ready", q_state: 2 },
    { id: 4, course_id: 1, document_id: 2, idx: 3, page_from: 1, page_to: 5, title: "Preiselastizität der Nachfrage", summary: "Wie stark die nachgefragte Menge reagiert, wenn sich der Preis ändert.", status: "ready", q_state: 2 },
    { id: 5, course_id: 1, document_id: 2, idx: 4, page_from: 6, page_to: 10, title: "Kreuzpreis- und Einkommenselastizität", summary: "Substitute, Komplemente und was ein höheres Einkommen mit der Nachfrage macht.", status: "ready", q_state: 1 },
    { id: 6, course_id: 1, document_id: 2, idx: 5, page_from: 11, page_to: 15, title: "Seiten 11-15", summary: "", status: "pending", q_state: 0 },
    { id: 7, course_id: 1, document_id: 2, idx: 6, page_from: 16, page_to: 19, title: "Seiten 16-19", summary: "", status: "pending", q_state: 0 },
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
  ].map(([unit_id, kind, prompt, options, answer, why], i) => ({ id: i + 1, unit_id, kind, prompt, options, answer, why, box: i % 4 === 1 ? 2 : 0, due_at: i % 4 === 1 ? now() - 10 : 0 })),
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
    due: db.questions.filter((x) => x.box > 0 && x.due_at <= now()).length,
    week,
    active_days: [...new Set(db.lessons.map((l) => new Date(l.at * 1000).toLocaleDateString("sv-SE")))],
  };
}
const shuffle = (a) => [...a].sort(() => Math.random() - 0.5);
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
      return { ...c, units: us.length, ready: us.filter((u) => u.status === "ready").length, documents: db.documents.filter((d) => d.course_id === c.id).map((d) => d.name) };
    }),
  create_course: ({ name, examDate }) => {
    const id = db.nextId++;
    db.courses.push({ id, name, exam_date: examDate, created_at: now() });
    return id;
  },
  update_course: ({ id, name, examDate }) => Object.assign(db.courses.find((c) => c.id === id), { name, exam_date: examDate }),
  delete_course: ({ id }) => {
    db.courses = db.courses.filter((c) => c.id !== id);
    db.units = db.units.filter((u) => u.course_id !== id);
  },
  import_document: ({ courseId, name, pages }) => {
    const text = pages.filter((p) => p.trim());
    if (!text.length) throw "In diesem PDF wurde kein Text gefunden. Ist es eingescannt?";
    const docId = db.nextId++;
    db.documents.push({ id: docId, course_id: courseId, name });
    const n = Math.max(1, Math.round(text.join("").length / 5000));
    let idx = db.units.filter((u) => u.course_id === courseId).length;
    for (let i = 0; i < n; i++) db.units.push({ id: db.nextId++, course_id: courseId, document_id: docId, idx: idx++, page_from: i * 5 + 1, page_to: i * 5 + 5, title: `Seiten ${i * 5 + 1}-${i * 5 + 5}`, summary: "", status: "pending", q_state: 0 });
    return { units: n };
  },
  course_path: ({ courseId }) =>
    db.units
      .filter((u) => u.course_id === courseId)
      .map((u) => {
        const qs = db.questions.filter((x) => x.unit_id === u.id);
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
  start_lesson: ({ unitId }) => {
    const pool = unitId == null ? db.questions.filter((x) => x.box > 0 && x.due_at <= now()) : db.questions.filter((x) => x.unit_id === unitId);
    if (!pool.length && unitId != null) return { ready: false, failed: false, exercises: [] };
    const ex = shuffle(pool)
      .slice(0, 8)
      .map((x) => ({ id: x.id, kind: x.kind, prompt: x.prompt, options: x.kind === "true_false" ? x.options : shuffle(x.options), pairs: [] }));
    const terms = shuffle(db.terms.filter((t) => (unitId == null ? pool.some((p) => p.unit_id === t.unit_id) : t.unit_id === unitId))).slice(0, 4);
    if (terms.length >= 3) ex.splice(Math.min(3, ex.length), 0, { id: null, kind: "match", prompt: "Ordne die Begriffe zu", options: [], pairs: terms.map((t) => [t.term, t.definition]) });
    return { ready: true, failed: false, exercises: ex };
  },
  answer: ({ questionId, given, retry }) => {
    const x = db.questions.find((y) => y.id === questionId);
    const correct = norm(given) === norm(x.answer);
    if (!retry) {
      x.box = correct ? Math.min(5, x.box + 1) : 1;
      x.due_at = now() + [0, 1, 3, 7, 14, 30][x.box] * DAY;
    }
    return { correct, expected: x.answer, why: x.why };
  },
  stats: () => stats(),
  finish_lesson: ({ unitId, correct, total }) => {
    const xp = 2 * correct + 5 + (total > 0 && correct >= total ? 5 : 0);
    db.lessons.push({ unit_id: unitId, xp, at: now() });
    return { xp, stats: stats() };
  },
  set_goal: ({ xp }) => (db.goal = xp),
  retry_questions: () => null,
};

class Channel {
  onmessage = null;
}

window.__TAURI__ = {
  core: {
    Channel,
    async invoke(cmd, args = {}) {
      await wait(60);
      if (!handlers[cmd]) throw `Unbekannter Befehl: ${cmd}`;
      return structuredClone(await handlers[cmd](args));
    },
  },
  event: {
    async listen(ev, fn) {
      (listeners[ev] ||= []).push(fn);
      return () => (listeners[ev] = listeners[ev].filter((f) => f !== fn));
    },
  },
};
