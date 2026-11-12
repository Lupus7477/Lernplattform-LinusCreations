// Lernkarte einer Einheit: Zusammenfassung, Begriffe, ausführliche Erklärung, „Frag die KI“.
import { call, stream } from "./api.js";
import { docName, h, icon, markdown, openSheet } from "./ui.js";
import { state } from "./state.js";

export async function render(id) {
  const u = await call("unit", { unitId: Number(id) });
  const explainBox = h("div.stack");

  const showExplanation = (text) => explainBox.replaceChildren(h("h2", "Ausführlich erklärt"), markdown(text));
  if (u.explanation) showExplanation(u.explanation);
  else {
    const btn = h("button.btn.secondary", { type: "button" }, icon("sparkle"), "Ausführlich erklären");
    btn.addEventListener("click", () => explain(u.id, explainBox, showExplanation));
    explainBox.append(btn);
  }

  const terms = u.terms.length
    ? h(
        "div.card",
        h("h2", "Wichtige Begriffe"),
        h("div.terms", u.terms.map((t) => h("div.term", h("span", h("span.mark", t.term)), h("span", t.definition)))),
      )
    : null;

  return h(
    "div.page",
    h("div.row", h("a.icon-btn", { href: "#/learn", "aria-label": "Zurück zum Lernpfad" }, icon("arrow-left"))),
    h("div.unit-head", h("h1", u.title), h("p.muted", `Aus ${docName(u.document)}, Seiten ${u.page_from}-${u.page_to}`)),
    u.summary ? h("p", { style: { "font-size": "1.08rem" } }, u.summary) : null,
    h("a.btn.block", { href: `#/lesson/${u.id}` }, "Lektion starten"),
    terms,
    explainBox,
    h("button.btn.secondary", { type: "button", onclick: () => openChat(u.id, u.title) }, icon("chat-circle-dots"), "Frag die KI"),
  );
}

async function explain(unitId, box, done) {
  if (state.ai?.state !== "ready") {
    box.replaceChildren(h("p.muted", "Die KI wird noch eingerichtet. Sobald der Download fertig ist, kann sie erklären."));
    return;
  }
  let text = "";
  const live = h("div.markdown.typing", { "aria-live": "polite" });
  box.replaceChildren(h("h2", "Ausführlich erklärt"), live);
  let queued = false;
  try {
    const final = await stream("explain", { unitId }, (piece) => {
      text += piece;
      if (queued) return;
      queued = true;
      // Markdown nur ~8× pro Sekunde neu setzen statt bei jedem Token
      setTimeout(() => {
        queued = false;
        live.replaceChildren(...markdown(text).childNodes);
      }, 120);
    });
    done(final);
  } catch (e) {
    box.replaceChildren(h("p.muted", String(e)));
  }
}

/**
 * Chat-Sheet. `seed` startet direkt mit einer Frage: { shown: Text in der Blase, sent: Text an die KI }.
 */
export function openChat(unitId, title, seed) {
  const msgs = [];
  const list = h("div.msgs", { "aria-live": "polite" });
  const input = h("textarea.input", { rows: 1, placeholder: "Was ist dir unklar?", "aria-label": "Deine Frage" });
  const send = h("button.btn", { type: "submit", "aria-label": "Senden" }, icon("paper-plane-right"));
  const form = h("form.chat-form", input, send);

  if (!seed) list.append(h("p.muted", `Frag alles zu „${title}“. Die KI antwortet nur mit dem Inhalt dieser Einheit.`));

  async function ask(shown, sent = shown) {
    msgs.push({ role: "user", content: sent });
    list.querySelector("p.muted")?.remove();
    list.append(h("div.msg.user", shown));
    const bubble = h("div.msg.assistant.typing");
    list.append(bubble);
    bubble.scrollIntoView({ block: "end" });
    send.disabled = true;
    if (state.ai?.state !== "ready") {
      bubble.classList.remove("typing");
      bubble.textContent = "Die KI wird noch eingerichtet. Versuch es gleich nochmal.";
      send.disabled = false;
      return;
    }
    let text = "";
    try {
      text = await stream("ask", { unitId, messages: msgs }, (p) => {
        text += p;
        bubble.textContent = text;
        bubble.scrollIntoView({ block: "end" });
      });
      bubble.replaceChildren(...markdown(text).childNodes);
      msgs.push({ role: "assistant", content: text });
    } catch (e) {
      bubble.textContent = String(e);
    }
    bubble.classList.remove("typing");
    send.disabled = false;
  }

  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const q = input.value.trim();
    if (!q || send.disabled) return;
    input.value = "";
    ask(q);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      form.requestSubmit();
    }
  });

  openSheet({ title: "Frag die KI", body: list, foot: form });
  if (seed) ask(seed.shown, seed.sent);
}
