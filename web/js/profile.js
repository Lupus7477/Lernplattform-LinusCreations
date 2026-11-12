// Profil: Streak, Kalender, Woche, Einstellungen, KI auf diesem Gerät.
import { call } from "./api.js";
import { applyTheme, gb, h, icon, pref } from "./ui.js";
import { aiNotice, emit, loadStats, state, subscribe } from "./state.js";

const DOW = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const iso = (d) => d.toLocaleDateString("sv-SE"); // JJJJ-MM-TT in lokaler Zeit

export async function render() {
  const page = h("div.page");
  const [stats, tiers] = await Promise.all([loadStats(), call("ai_tiers")]);
  const draw = () => page.replaceChildren(...content(stats, tiers));
  draw();
  page.cleanup = subscribe((kind) => kind === "ai" && draw());
  return page;
}

function content(s, tiers) {
  return [
    h("h1", "Profil"),
    h(
      "div.card",
      h("div.hero-stat", icon("fire-fill"), h("div", h("b", String(s.streak)), h("p.muted", s.streak === 1 ? "Tag in Folge" : "Tage in Folge"))),
      h("p.muted.num", `Insgesamt ${s.total_xp} XP aus ${s.lessons} ${s.lessons === 1 ? "Lektion" : "Lektionen"}.`),
    ),
    h("div.card", h("h2", "Letzte 6 Wochen"), calendar(s.active_days)),
    h("div.card", h("h2", "Diese Woche"), week(s.week, s.goal)),
    settings(s),
    aiCard(tiers),
  ];
}

function calendar(active) {
  const on = new Set(active);
  const today = new Date();
  today.setHours(12, 0, 0, 0);
  const end = new Date(today);
  end.setDate(end.getDate() + ((7 - ((end.getDay() + 6) % 7) - 1) % 7)); // bis Sonntag dieser Woche
  const start = new Date(end);
  start.setDate(start.getDate() - 41);
  const cells = DOW.map((d) => h("span.dow", d));
  for (let d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
    const key = iso(d);
    const isOn = on.has(key);
    const future = d > today;
    cells.push(
      h(
        `span.day${isOn ? ".on" : ""}${key === iso(today) ? ".today" : ""}`,
        { title: d.toLocaleDateString("de-DE", { weekday: "long", day: "numeric", month: "long" }), style: future ? { opacity: "0.35" } : undefined, "aria-label": `${d.toLocaleDateString("de-DE")}${isOn ? ", gelernt" : ""}` },
        isOn ? icon("fire-fill") : String(d.getDate()),
      ),
    );
  }
  return h("div.calendar", cells);
}

function week(days, goal) {
  const max = Math.max(goal, ...days.map((d) => d.xp), 1);
  const cols = days.map((d) => {
    const wd = DOW[(new Date(`${d.day}T12:00:00`).getDay() + 6) % 7];
    return h(`div.col${d.xp ? "" : ".zero"}`, { title: `${d.xp} XP` }, h("i", { style: { "--h": String(d.xp / max) } }), h("span", wd));
  });
  return h(
    "div",
    h("div.week", { role: "img", "aria-label": `XP der letzten 7 Tage: ${days.map((d) => d.xp).join(", ")}. Tagesziel ${goal} XP.` }, cols, h("span.goal", { style: { bottom: `${24 + 4 + (goal / max) * 100}px` } })),
    h("p.muted", { style: { "margin-top": "10px" } }, `Gestrichelte Linie: dein Tagesziel von ${goal} XP.`),
  );
}

function segmented(options, value, onPick, label) {
  const btns = options.map(([v, text]) =>
    h("button", { type: "button", "aria-pressed": String(v === value) }, text),
  );
  btns.forEach((b, i) =>
    b.addEventListener("click", () => {
      btns.forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
      onPick(options[i][0]);
    }),
  );
  return h("div.segmented", { role: "group", "aria-label": label }, btns);
}

function settings(s) {
  const soundSwitch = h("input.switch", { type: "checkbox", id: "sound", checked: pref.get("sound", "on") === "on" });
  soundSwitch.addEventListener("change", () => pref.set("sound", soundSwitch.checked ? "on" : "off"));
  return h(
    "div.card",
    h("h2", "Einstellungen"),
    h(
      "div.field",
      h("label", "Tagesziel"),
      segmented(
        [10, 20, 30, 50].map((x) => [x, `${x} XP`]),
        s.goal,
        async (xp) => {
          await call("set_goal", { xp });
          await loadStats();
          emit("stats", state.stats);
        },
        "Tagesziel",
      ),
      h("span.hint", "Eine Lektion bringt etwa 15 bis 30 XP."),
    ),
    h("div.row.between", h("label", { for: "sound", style: { "font-weight": "700" } }, "Töne bei Antworten"), soundSwitch),
    h(
      "div.field",
      h("label", "Darstellung"),
      segmented(
        [
          ["system", "System"],
          ["light", "Hell"],
          ["dark", "Dunkel"],
        ],
        pref.get("theme", "system"),
        (mode) => {
          pref.set("theme", mode);
          applyTheme(mode);
        },
        "Darstellung",
      ),
    ),
  );
}

function aiCard(tiers) {
  const ai = state.ai;
  const line =
    ai?.state === "ready" ? (ai.loaded ? "Bereit und gerade im Speicher." : "Bereit. Ruht, bis du sie brauchst.") : ai?.state === "downloading" ? `Wird geladen: ${gb(ai.done)} von ${gb(ai.total)} GB` : ai?.state === "error" ? "Nicht verfügbar." : "Wird vorbereitet.";
  return h(
    "div.card",
    h("h2", "KI auf diesem Gerät"),
    h("p", icon("cpu"), " ", line),
    aiNotice(),
    h(
      "div.stack",
      { role: "radiogroup", "aria-label": "Modell" },
      tiers.map((t) =>
        h(
          "label.tier",
          h("input", {
            type: "radio",
            name: "tier",
            checked: ai?.tier === t.id,
            onchange: () => call("ai_set_tier", { tier: t.id }),
          }),
          h("b", t.label),
          h("small", t.id === "kompakt" ? "Für Handys und ältere Rechner. Schneller, etwas einfachere Fragen." : "Für Laptops, PCs und neue iPads. Bessere Fragen und Erklärungen."),
        ),
      ),
    ),
    h("p.muted", "Die KI läuft komplett auf diesem Gerät. Nach drei Minuten ohne Nutzung gibt sie den Arbeitsspeicher wieder frei. Ein Wechsel lädt das andere Modell einmalig herunter."),
  );
}
