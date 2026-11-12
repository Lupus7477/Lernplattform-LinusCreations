// Lernkarte einer Einheit: Zusammenfassung, Begriffe, Fragen, ausführliche Erklärung, „Frag die KI“.
import { call, stream } from "./api.js";
import { docName, h, icon, markdown, openSheet, segmented, toast } from "./ui.js";
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
    questionList(u.id),
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

// ---------- Fragen ansehen, bearbeiten, ausblenden ----------

/** Aufklappbare Liste aller Fragen der Einheit. Lädt erst beim ersten Öffnen. */
function questionList(unitId) {
  const list = h("div.q-list", { "aria-live": "polite" });
  const box = h("details.card.q-box", h("summary", h("h2", "Fragen dieser Einheit"), icon("caret-down")), list);
  let loaded = false;

  async function draw() {
    let qs;
    try {
      qs = await call("unit_questions", { unitId });
    } catch (e) {
      return list.replaceChildren(h("p.muted", String(e)));
    }
    if (!qs.length) return list.replaceChildren(h("p.muted", "Noch keine Fragen. Sie entstehen, sobald du die erste Lektion startest."));
    const hidden = qs.filter((q) => q.hidden).length;
    list.replaceChildren(
      h("p.muted", hidden ? `${qs.length - hidden} aktiv, ${hidden} ausgeblendet.` : "Unklar oder nicht prüfungsrelevant? Bearbeite oder blende die Frage aus."),
      ...qs.map((q) => {
        const toggle = h("button.btn.ghost", { type: "button" }, icon(q.hidden ? "eye" : "eye-slash"), q.hidden ? "Einblenden" : "Ausblenden");
        toggle.addEventListener("click", async () => {
          await call("hide_question", { questionId: q.id, hidden: !q.hidden });
          draw();
        });
        return h(
          `div.q-item${q.hidden ? ".is-hidden" : ""}`,
          h("p.q-text", q.prompt),
          h("p.muted", "Richtig: ", h("b", q.answer)),
          h("div.row", h("button.btn.ghost", { type: "button", onclick: () => editQuestion(q, draw) }, icon("pencil-simple"), "Bearbeiten"), toggle),
        );
      }),
    );
  }
  box.addEventListener("toggle", () => {
    if (box.open && !loaded) {
      loaded = true;
      list.replaceChildren(h("div.skeleton", { style: { height: "62px" } }));
      draw();
    }
  });
  return box;
}

const KIND_LABEL = { mc: "Multiple Choice", true_false: "Richtig oder falsch", fill_blank: "Lückentext" };
const same = (a, b) => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * Formular-Sheet für eine Frage. Änderungen gelten für alle im Kurs.
 * `onSaved(frage)` bekommt die gespeicherte Fassung vom Kern (bereinigt).
 */
export function editQuestion(q, onSaved) {
  const err = h("p.form-error", { id: "q-error", role: "alert", hidden: true });
  const prompt = h("textarea.input", { id: "q-prompt", rows: 3, value: q.prompt, "aria-describedby": "q-error" });
  const why = h("textarea.input", { id: "q-why", rows: 2, value: q.why, placeholder: "Ein Satz, warum die Antwort stimmt" });

  let answers; // liefert { options, answer }
  let answerField;
  if (q.kind === "true_false") {
    let pick = q.answer;
    answerField = h("div.field", h("label", "Richtige Antwort"), segmented([["Richtig", "Richtig"], ["Falsch", "Falsch"]], q.answer, (v) => (pick = v), "Richtige Antwort"));
    answers = () => ({ options: ["Richtig", "Falsch"], answer: pick });
  } else {
    const rows = [0, 1, 2, 3].map((i) => {
      const o = q.options[i] ?? "";
      const radio = h("input", { type: "radio", name: "q-correct", checked: o !== "" && same(o, q.answer), "aria-label": `Antwort ${i + 1} ist richtig` });
      const text = h("input.input", { value: o, autocomplete: "off", "aria-label": `Antwort ${i + 1}`, placeholder: q.kind === "fill_blank" && i > 1 ? "optional" : "" });
      return { radio, text, el: h("div.opt-row", radio, text) };
    });
    answerField = h("div.field", h("label", "Antworten"), h("span.hint", "Markiere die richtige."), ...rows.map((r) => r.el));
    answers = () => {
      const filled = rows.filter((r) => r.text.value.trim());
      return { options: filled.map((r) => r.text.value.trim()), answer: rows.find((r) => r.radio.checked)?.text.value.trim() ?? "" };
    };
  }

  const save = h("button.btn.block", { type: "button" }, "Speichern");
  const s = openSheet({
    title: "Frage bearbeiten",
    body: [
      h("p.muted", q.shared ? `${KIND_LABEL[q.kind]}. Die Änderung gilt für alle in diesem Kurs.` : KIND_LABEL[q.kind]),
      h(
        "div.field",
        h("label", { for: "q-prompt" }, q.kind === "fill_blank" ? "Satz mit Lücke" : q.kind === "true_false" ? "Aussage" : "Frage"),
        prompt,
        q.kind === "fill_blank" ? h("span.hint", "Markiere die Lücke mit drei Unterstrichen ___") : null,
      ),
      answerField,
      h("div.field", h("label", { for: "q-why" }, "Erklärung (optional)"), why),
      err,
    ],
    foot: save,
  });

  function fail(text) {
    err.textContent = text;
    err.hidden = false;
    prompt.setAttribute("aria-invalid", "true");
  }

  save.addEventListener("click", async () => {
    const { options, answer } = answers();
    const p = prompt.value.trim();
    // Schnelle Prüfung hier, die verbindliche macht der Kern.
    if (p.length < 8) return fail("Die Frage braucht mindestens 8 Zeichen.");
    if (q.kind === "fill_blank" && !p.includes("__")) return fail("Im Satz fehlt die Lücke ___.");
    if (q.kind === "mc" && options.length < 4) return fail("Multiple Choice braucht 4 Antworten.");
    if (q.kind === "fill_blank" && options.length < 2) return fail("Gib mindestens 2 Antworten zur Auswahl.");
    if (!answer) return fail("Markiere die richtige Antwort.");
    save.disabled = true;
    try {
      const saved = await call("update_question", { questionId: q.id, prompt: p, options, answer, why: why.value });
      s.close();
      toast(q.shared ? "Gespeichert. Gilt für alle im Kurs." : "Gespeichert.");
      onSaved?.(saved);
    } catch (e) {
      fail(String(e));
      save.disabled = false;
    }
  });
}
