// Wiederholen: fällige Fragen über alle Kurse + Begriffe durchsuchen.
import { call } from "./api.js";
import { h, icon } from "./ui.js";
import { loadStats, subscribe } from "./state.js";

export async function render() {
  const stats = await loadStats();
  const results = h("div.terms", { "aria-live": "polite" });
  const search = h("input.input", { id: "term-search", type: "search", autocomplete: "off", placeholder: "z. B. Elastizität" });

  let timer;
  async function find() {
    const items = await call("search_terms", { query: search.value });
    results.replaceChildren(
      ...(items.length
        ? items.map((t) => h("div.term", h("span", h("span.mark", t.term)), h("span", t.definition), h("a.from", { href: `#/unit/${t.unit_id}` }, `aus „${t.unit_title}“`)))
        : [h("p.muted", search.value ? "Kein Begriff passt zu deiner Suche." : "Begriffe erscheinen hier, sobald ein Skript vorbereitet ist.")]),
    );
  }
  search.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(find, 200);
  });
  find();

  const due = stats.due;
  const dueCard = due
    ? h(
        "div.card",
        h("div.hero-stat", icon("cards", ""), h("div", h("b", String(due)), h("p.muted", due === 1 ? "Frage ist heute fällig" : "Fragen sind heute fällig"))),
        h("a.btn.block", { href: "#/lesson/review" }, "Wiederholung starten"),
      )
    : h("div.empty", h("div.art", icon("seal-check-fill")), h("h2", "Alles wiederholt"), h("p", "Fragen kommen zur Wiederholung zurück, kurz bevor du sie vergessen würdest."));
  dueCard.querySelector(".hero-stat .icon")?.style.setProperty("color", "var(--royal)");

  const page = h(
    "div.page",
    h("h1", "Wiederholen"),
    dueCard,
    h("div.card", h("h2", "Begriffe"), h("div.field", h("label", { for: "term-search" }, "Begriff suchen"), search), results),
  );
  page.cleanup = subscribe((kind) => kind === "course-updated" && find());
  return page;
}
