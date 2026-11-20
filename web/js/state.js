// Gemeinsamer Zustand der App + Bausteine, die mehrere Ansichten nutzen.
import { call } from "./api.js";
import { gb, h, icon, pref, toast } from "./ui.js";

export const state = {
  /** { mode: "local" | "server", user: { name, admin } | null } */
  session: null,
  ai: null,
  courses: [],
  stats: null,
  courseId: null,
};

const listeners = new Set();
/** Ansichten melden sich hier für Aktualisierungen an (KI-Status, Kursfortschritt, Statistik). */
export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
export function emit(kind, payload) {
  for (const fn of [...listeners]) fn(kind, payload);
}

export function currentCourse() {
  return state.courses.find((c) => c.id === state.courseId) || state.courses[0] || null;
}

export function selectCourse(id) {
  state.courseId = id;
  pref.set("course", String(id));
  emit("course-selected", id);
}

export async function loadCourses() {
  state.courses = await call("courses");
  const saved = Number(pref.get("course", "0"));
  if (!state.courses.some((c) => c.id === state.courseId)) {
    state.courseId = state.courses.find((c) => c.id === saved)?.id ?? state.courses[0]?.id ?? null;
  }
  return state.courses;
}

let statsSeq = 0;
export async function loadStats() {
  const mine = ++statsSeq;
  const stats = await call("stats", { courseId: currentCourse()?.id ?? null }); // Level und Titel des gewählten Kurses
  if (stats.notice) toast(stats.notice); // gerade eingesetzter Pausentag, kommt nur einmal
  if (mine !== statsSeq) return stats; // inzwischen neu angefragt (Kurswechsel): die neuere Antwort gilt
  state.stats = stats;
  emit("stats", state.stats);
  return state.stats;
}

/** Hinweis, solange die KI nicht bereit ist. Leer, wenn alles läuft. */
/** „0,4 von 1,3 GB“ als eigener Knoten, den `setAiProgress` austauscht. */
export const aiProgress = (ai) => h("span.ai-progress", progressText(ai));
const progressText = (ai) => `${gb(ai.done || 0)} von ${gb(ai.total || 1)} GB`;

/** Während des Downloads ändert sich nur der Fortschritt: Zahl und Balken tauschen, statt die Seiten neu zu bauen (sonst gehen Eingaben verloren). */
export function setAiProgress(ai) {
  document.querySelectorAll(".ai-progress").forEach((el) => (el.textContent = progressText(ai)));
  document.querySelectorAll(".ai-bar").forEach((el) => el.style.setProperty("--p", String((ai.done || 0) / (ai.total || 1))));
}

export function aiNotice(ai = state.ai) {
  if (!ai || ai.state === "ready") return null;
  if (ai.state === "error") {
    return h(
      "div.notice.error",
      { role: "alert" },
      icon("warning"),
      h("b", "Die KI konnte nicht geladen werden"),
      h("span.muted", ai.message),
      h("div.actions", h("button.btn.secondary", { type: "button", onclick: () => call("ai_retry") }, icon("arrow-clockwise"), "Erneut versuchen")),
    );
  }
  // Die App lädt erst auf Wunsch: 1–3 GB, unterwegs vielleicht über mobile Daten. Auf dem Server darf das nur ein Admin.
  if (ai.state === "missing") {
    const may = state.session?.mode !== "server" || state.session.user?.admin;
    return h(
      "div.notice",
      { role: "status" },
      icon("download-simple"),
      h("b", "KI herunterladen"),
      h("span.muted.num", `Einmalig ${gb(ai.size)} GB, am besten im WLAN. Danach schreibt sie Fragen und Erklärungen offline auf diesem Gerät.`),
      may ? h("div.actions", h("button.btn", { type: "button", onclick: (e) => ((e.currentTarget.disabled = true), call("ai_retry")) }, icon("download-simple"), `Herunterladen (${gb(ai.size)} GB)`)) : null,
    );
  }
  const done = ai.done || 0;
  const total = ai.total || 1;
  return h(
    "div.notice",
    { role: "status" },
    icon("download-simple"),
    h("b", "Die KI wird eingerichtet"),
    ai.state === "downloading" ? h("span.muted.num", aiProgress(ai), ". Einmalig, danach läuft alles offline auf diesem Gerät.") : h("span.muted.num", "Einen Moment, gleich geht es los."),
    h("div.bar", h("i.ai-bar", { style: { "--p": String(done / total) } })),
  );
}
