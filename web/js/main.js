import { call, on } from "./api.js";
import { applyTheme, h, icon } from "./ui.js";
import { currentCourse, emit, loadCourses, loadStats, selectCourse, state, subscribe } from "./state.js";
import * as path from "./path.js";
import * as review from "./review.js";
import * as add from "./import.js";
import * as profile from "./profile.js";
import * as unit from "./unit.js";
import * as battle from "./battle.js";
import { openLesson } from "./lesson.js";
import * as auth from "./auth.js";
import { appInfo, bootReport } from "./report.js";
import { syncReminder } from "./remind.js";

const views = { learn: path, review, add, profile, unit, battle };
const TABS = [
  ["learn", "path", "Lernen"],
  ["review", "cards", "Wiederholen"],
  ["battle", "crown-fill", "Battle"], // nur mit eigenem Server: braucht andere Konten
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
  if (!window.__TAURI__ && !(await servedByLernpfad())) await import("../dev/mock-tauri.js");
  document.body.insertAdjacentHTML("afterbegin", await sprite);

  try {
    state.session = await call("session");
  } catch (e) {
    if (window.__TAURI__) return offline(String(e?.message || e)); // App mit Server verbunden, Server weg
    throw e;
  }
  // Server ohne gültige Sitzung: nur die Anmeldung, keine Daten.
  if (state.session.mode === "server" && !state.session.user) return auth.show(state.session);
  addEventListener("lernpfad:logged-out", () => location.reload());

  [state.ai] = await Promise.all([call("ai_status"), loadCourses(), loadStats(), appInfo()]);
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
    state.ai = s;
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
  topbar.replaceChildren(
    courseEl,
    h("span.spacer"),
    h("span.chip.streak" + (s.streak ? "" : ".off"), { "aria-label": `Streak: ${s.streak} ${s.streak === 1 ? "Tag" : "Tage"}` }, icon("fire-fill"), h("span.num", String(s.streak))),
    h("span.chip", { "aria-label": `Heute ${s.xp_today} von ${s.goal} XP` }, h("span.ring", { style: { "--p": String(p) } }), h("span.num", `${s.xp_today} XP`)),
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

async function show(name, arg) {
  const same = mounted.name === name && mounted.arg === arg;
  mounted.el?.cleanup?.();
  const el = await views[name].render(arg);
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
