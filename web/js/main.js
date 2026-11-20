import { call, message, on } from "./api.js";
import { applyTheme, h, icon } from "./ui.js";
import { currentCourse, emit, loadCourses, loadStats, selectCourse, setAiProgress, state, subscribe } from "./state.js";
import * as path from "./path.js";
import * as review from "./review.js";
import * as add from "./import.js";
import * as profile from "./profile.js";
import * as unit from "./unit.js";
import * as battle from "./battle.js";
import { openLesson } from "./lesson.js";
import * as auth from "./auth.js";
import { appInfo, bootReport, fail } from "./report.js";
import { syncReminder } from "./remind.js";

const views = { learn: path, review, add, profile, unit, battle };
const TABS = [
  ["learn", "path", "Lernen"],
  ["review", "cards", "Wiederholen"],
  ["battle", "crown", "Battle"], // nur mit eigenem Server: braucht andere Konten
  ["add", "plus-circle", "Hinzufügen"],
  ["profile", "user-circle", "Profil"],
];

let main;
let topbar;
let tabs;
let mounted = { name: null, arg: null, el: null };
let lesson = null;

/** Läuft die Seite vom eigenen Server? Sonst (z. B. python -m http.server) → Beispieldaten. */
async function servedByLernpfad() {
  const r = await fetch("api/rpc/session", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" }).catch(() => null);
  return !!r && (r.ok || r.status === 401);
}

async function boot() {
  applyTheme();
  const sprite = fetch("icons.svg").then((r) => r.text());
  // Beispieldaten nur bei der Entwicklung auf diesem Rechner: Ein echter Server mit 502/503 soll einen Fehler zeigen, keine Fake-Kurse
  const dev = ["localhost", "127.0.0.1"].includes(location.hostname);
  if (!window.__TAURI__ && dev && !(await servedByLernpfad())) await import("../dev/mock-tauri.js");
  document.body.insertAdjacentHTML("afterbegin", await sprite);

  try {
    state.session = await call("session");
  } catch (e) {
    if (window.__TAURI__) return offline(message(e)); // App mit Server verbunden, Server weg
    throw e;
  }
  // Server ohne gültige Sitzung: nur die Anmeldung, keine Daten.
  if (state.session.mode === "server" && !state.session.user) return auth.show(state.session);
  addEventListener("lernpfad:logged-out", () => location.reload());

  // Erst die Kurse, dann die Statistik: Level und Titel gelten für den gewählten Kurs
  [state.ai] = await Promise.all([call("ai_status"), loadCourses().then(loadStats), appInfo()]);
  syncReminder(state.stats);

  topbar = h("header.topbar");
  main = h("main", { id: "main", tabindex: "-1" });
  tabs = h(
    "nav.tabbar",
    { "aria-label": "Hauptnavigation" },
    h("div.brand", icon("path"), "Lernpfad"),
    TABS.filter(([name]) => name !== "battle" || state.session.mode === "server").map(([name, ic, label]) => h("a.tab", { href: `#/${name}`, "data-tab": name }, icon(ic), h("span", label))),
  );
  document.getElementById("app").replaceChildren(topbar, main, tabs);
  renderTopbar();
  subscribe((kind, payload) => {
    if (kind === "stats" || kind === "course-selected" || kind === "courses") renderTopbar();
    if (kind === "stats") syncReminder(payload); // Tagesziel erreicht → heute keine Erinnerung mehr
  });

  on("ai-status", (s) => {
    const progress = s.state === "downloading" && state.ai?.state === "downloading" && s.tier === state.ai.tier;
    state.ai = s;
    if (progress) return setAiProgress(s); // pro Prozent ein Ereignis: nicht jedes Mal Hinzufügen, Lernen und Profil neu bauen
    emit("ai", s);
  });
  let pending;
  on("course-updated", (id) => {
    clearTimeout(pending);
    pending = setTimeout(async () => {
      await loadCourses();
      emit("course-updated", id);
    }, 300);
  });

  addEventListener("hashchange", route);
  route();
}

/** App ist mit dem eigenen Server verbunden, der gerade nicht antwortet: lokal geht immer. */
function offline(message) {
  const local = h("button.btn.block", { type: "button" }, icon("path"), "Lokal weiterlernen");
  local.addEventListener("click", () => call("server_disconnect").then(() => location.reload()));
  document.getElementById("app").replaceChildren(
    h(
      "main.auth",
      h(
        "div.card",
        h("div.auth-brand", icon("hard-drives"), "Eigener Server"),
        h("h1", "Server nicht erreichbar"),
        h("p.muted", message),
        h("p.muted", "Deine Daten auf diesem Gerät sind unverändert. Du kannst lokal weiterlernen und dich später wieder verbinden."),
        local,
        h("button.btn.secondary.block", { type: "button", onclick: () => location.reload() }, icon("arrow-clockwise"), "Erneut versuchen"),
      ),
    ),
  );
}

function renderTopbar() {
  const s = state.stats || { streak: 0, xp_today: 0, goal: 20 };
  const course = currentCourse();
  let courseEl;
  if (state.courses.length > 1) {
    const select = h(
      "select",
      { "aria-label": "Kurs wählen", onchange: (e) => selectCourse(Number(e.target.value)) },
      state.courses.map((c) => h("option", { value: String(c.id), selected: c.id === course?.id }, c.name)),
    );
    courseEl = h("label.chip.course-pill", { style: { position: "relative" } }, h("span", course?.name), icon("caret-down"), select);
  } else {
    courseEl = h("span.mark-brand", icon("path"), "Lernpfad"); // ein Kurs: Name steht schon als Überschrift
  }
  const p = Math.min(1, s.xp_today / Math.max(1, s.goal));
  // Fällige Fragen als Zahl am Tab „Wiederholen“
  const reviewTab = tabs.querySelector('[data-tab="review"]');
  if (s.due > 0) reviewTab.dataset.due = s.due > 99 ? "99+" : String(s.due);
  else delete reviewTab.dataset.due;
  reviewTab.setAttribute("aria-label", s.due > 0 ? `Wiederholen, ${s.due} fällig` : "Wiederholen");
  topbar.replaceChildren(
    courseEl,
    h("span.spacer"),
    h("span.chip.streak" + (s.streak ? "" : ".off"), { role: "img", "aria-label": `Streak: ${s.streak} ${s.streak === 1 ? "Tag" : "Tage"}` }, icon("fire-fill"), h("span.num", String(s.streak))),
    h(
      "span.chip",
      { role: "img", "aria-label": `Heute ${s.xp_today} von ${s.goal} XP`, title: `Tagesziel: heute ${s.xp_today} von ${s.goal} XP über alle Kurse, beginnt um 0 Uhr neu` },
      h("span.ring", { style: { "--p": String(p) } }),
      h("span.num", `${s.xp_today}/${s.goal} XP`),
    ),
  );
}

function parse() {
  const [name = "learn", arg] = location.hash.replace(/^#\/?/, "").split("/");
  return { name: name || "learn", arg };
}

async function route() {
  const { name, arg } = parse();
  if (name === "lesson") {
    // #/lesson/<Einheit>, #/lesson/review (fällige), #/lesson/wrong-<Kurs> (falsche), #/lesson/exam-<Kurs> (Prüfungstraining) oder #/lesson/battle-<Battle>
    const wrongCourse = arg?.startsWith("wrong-") ? Number(arg.slice(6)) : null;
    const examCourse = arg?.startsWith("exam-") ? Number(arg.slice(5)) : null;
    const battleId = arg?.startsWith("battle-") ? Number(arg.slice(7)) : null;
    const base = battleId ? "battle" : arg === "review" || wrongCourse || examCourse ? "review" : "learn";
    if (mounted.name !== base) await show(base);
    lesson?.close();
    lesson = openLesson({ unitId: base === "learn" ? Number(arg) : null, wrongCourse, examCourse, battleId }, () => {
      lesson = null;
      location.replace(`#/${base}`);
    });
    return;
  }
  if (lesson) {
    lesson.close();
    lesson = null;
  }
  if (!views[name]) return location.replace("#/learn");
  await show(name, arg);
}

let showing = 0;
async function show(name, arg) {
  const n = ++showing;
  const same = mounted.name === name && mounted.arg === arg;
  mounted.el?.cleanup?.();
  mounted.el = null;
  let el;
  try {
    el = await views[name].render(arg);
  } catch (e) {
    fail(e); // Toast mit „Melden“ wie bei jedem anderen Fehler
    const retry = h("button.btn.secondary", { type: "button", onclick: () => show(name, arg) }, icon("arrow-clockwise"), "Erneut versuchen");
    el = h("div.page", h("div.notice.error", { role: "alert" }, icon("warning"), h("b", "Diese Seite konnte nicht geladen werden"), h("span.muted", message(e)), h("div.actions", retry)));
  }
  // Inzwischen schon woanders hin: die langsamere Ansicht darf die neuere nicht überdecken
  if (n !== showing) return el.cleanup?.();
  mounted = { name, arg, el };
  main.replaceChildren(el);
  const tab = name === "unit" ? "learn" : name;
  tabs.querySelectorAll(".tab").forEach((t) => (t.dataset.tab === tab ? t.setAttribute("aria-current", "page") : t.removeAttribute("aria-current")));
  if (!same) {
    scrollTo(0, 0);
    main.focus({ preventScroll: true });
  }
}

boot().catch((e) => {
  const message = String(e?.stack || e?.message || e);
  const report = h("div.actions", h("button.btn.secondary", { type: "button", onclick: () => bootReport(message) }, icon("warning"), "Fehler melden"));
  document.getElementById("app").replaceChildren(h("div.page", h("div.notice.error", icon("warning"), h("b", "Die App konnte nicht starten"), h("span", String(e?.message || e)), report)));
  console.error(e);
});
