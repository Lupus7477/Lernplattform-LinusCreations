// Battle: live gegen andere auf dem Server. Der Code wählt die Aufgaben fair aus, Bestenliste pro Kurs und für den ganzen Server.
import { call } from "./api.js";
import { h, icon, pref, segmented } from "./ui.js";
import { currentCourse, loadCourses, selectCourse, state } from "./state.js";
import { fail } from "./report.js";

export async function render() {
  await loadCourses();
  if (!state.courses.length) {
    return h(
      "div.page",
      h("div.empty", h("div.art", icon("crown-fill")), h("h1", "Erst ein Skript, dann ein Battle"), h("p", "Für ein Battle brauchst du einen Kurs mit Fragen."), h("a.btn", { href: "#/add" }, icon("upload-simple"), "Skript hinzufügen")),
    );
  }

  const select = h(
    "select.input",
    { id: "battle-course", onchange: (e) => (selectCourse(Number(e.target.value)), board()) },
    state.courses.map((c) => h("option", { value: String(c.id), selected: c.id === currentCourse()?.id }, c.name)),
  );
  const courseId = () => Number(select.value);
  const go = (id) => (location.hash = `#/lesson/battle-${id}`);
  const active = h("div");
  const open = h("div.stack", { "aria-live": "polite" });
  const boardTitle = h("h2");
  const ranks = h("ol.ranks");
  let scope = pref.get("battle-board", "course"); // course | server
  const scopeBtns = segmented(
    [
      ["course", "Dieser Kurs"],
      ["server", "Ganzer Server"],
    ],
    scope,
    (v) => {
      scope = v;
      pref.set("battle-board", v);
      board();
    },
    "Bestenliste für",
  );

  async function lobby() {
    if (document.querySelector(".lesson")) return; // Battle läuft darüber: die Lobby darunter sieht niemand
    const l = await call("battle_lobby").catch(() => null);
    if (!l) return;
    active.replaceChildren(
      l.active
        ? h("div.notice", { role: "status" }, icon("hourglass-medium"), h("b", "Dein Battle läuft noch"), h("div.actions", h("button.btn", { type: "button", onclick: () => go(l.active) }, "Weiterspielen")))
        : "",
    );
    open.replaceChildren(
      ...(l.open.length
        ? l.open.map((b) =>
            h(
              "div.wrong-row",
              h("div", h("b", b.name), h("p.muted", b.course ? `mit „${b.course}“` : "mit einem eigenen Kurs")),
              h("button.btn.secondary", { type: "button", onclick: () => call("battle_join", { battleId: b.id, courseId: courseId() }).then(() => go(b.id), fail) }, "Annehmen"),
            ),
          )
        : [h("p.muted", "Gerade fordert niemand heraus. Starte selbst eins, dann sehen es die anderen hier.")]),
    );
  }

  async function board() {
    const c = state.courses.find((x) => x.id === courseId());
    const server = scope === "server";
    boardTitle.textContent = server ? "Bestenliste · Server" : `Bestenliste · ${c?.name ?? ""}`;
    const list = await call("leaderboard", server ? {} : { courseId: courseId() }).catch(() => []);
    ranks.replaceChildren(
      ...(list.length
        ? list.map((r, i) =>
            h(
              `li.rank${r.me ? ".me" : ""}`,
              h("span.pos.num", i === 0 ? icon("crown-fill") : String(i + 1)),
              h("b", r.name),
              h("span.xp.num", `${r.xp} XP`),
              h("span.muted.num", r.battles ? `${r.wins} ${r.wins === 1 ? "Sieg" : "Siege"} · ${r.battles} ${r.battles === 1 ? "Battle" : "Battles"}` : "noch kein Battle"),
            ),
          )
        : [h("p.muted", server ? "Noch keine XP auf diesem Server." : "Noch keine XP in diesem Kurs.")]),
    );
  }

  const start = h("button.btn.block", { type: "button", onclick: () => call("battle_create", { courseId: courseId() }).then(go, fail) }, icon("users"), "Herausfordern");
  const page = h(
    "div.page",
    h("h1", "Battle"),
    active,
    h(
      "div.card",
      h("div.field", h("label", { for: "battle-course" }, "Dein Kurs"), select),
      h("p.hint", "Gewinnen bringt XP für die Bestenliste: 2 je richtige Antwort, 10 für den Sieg. Dazu zählen deine XP aus dem Lernen."),
      h("p.hint", "Ihr braucht keinen gemeinsamen Kurs: Jeder spielt mit seinem eigenen. Der Code sucht die Aufgaben so aus, dass ihr an jeder Stelle dieselbe Art mit gleich vielen Teilen bekommt. Mit demselben Kurs sind es genau dieselben Fragen."),
      start,
    ),
    h("div.card", h("h2", "Offene Herausforderungen"), open),
    h("div.card", boardTitle, scopeBtns, ranks),
  );
  lobby();
  board();
  // shortcut: Lobby per Abfrage alle 3 s; ein Server-Ereignis, falls viele gleichzeitig online sind.
  const timer = setInterval(lobby, 3000);
  page.cleanup = () => clearInterval(timer);
  return page;
}
