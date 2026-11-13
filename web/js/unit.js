// Lernkarte einer Einheit: Zusammenfassung, Begriffe, Fragen, ausführliche Erklärung, „Frag die KI“.
import { call, stream } from "./api.js";
import { docName, h, icon, markdown, openSheet, segmented, toast, zoomable } from "./ui.js";
import { state } from "./state.js";
import { fail } from "./report.js";

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

  const course = state.courses?.find((c) => c.id === u.course_id);
  return h(
    "div.page",
    h("div.row", h("a.icon-btn", { href: "#/learn", "aria-label": "Zurück zum Lernpfad" }, icon("arrow-left"))),
    h("div.unit-head", h("h1", u.title), h("p.muted", `Aus ${docName(u.document)}, Seiten ${u.page_from}-${u.page_to}`)),
    u.exam.length ? h("p.exam", icon("target"), `Prüfungsrelevant: ${u.exam.join(", ")}`) : null,
    u.summary ? h("p", { style: { "font-size": "1.08rem" } }, u.summary) : null,
    u.orga ? orgaNote(u, course?.mine) : null,
    h("a.btn.block", { href: `#/lesson/${u.id}` }, "Lektion starten"),
    terms,
    questionList(u.id, !!state.courses?.find((c) => c.id === u.course_id)?.shared),
    explainBox,
    h("button.btn.secondary", { type: "button", onclick: () => openChat(u.id, u.title) }, icon("chat-circle-dots"), "Frag die KI"),
  );
}

/** Seiten mit Organisatorischem, die nicht abgefragt werden; der Besitzer kann sie zurückholen. */
function orgaNote(u, mine) {
  const n = u.orga;
  const btn = mine ? h("button.btn.ghost", { type: "button" }, "Doch einbeziehen") : null;
  btn?.addEventListener("click", async () => {
    try {
      await call("include_orga", { unitId: u.id });
    } catch (e) {
      return fail(e);
    }
    btn.disabled = true;
    toast("Die KI bereitet den Abschnitt mit allen Seiten neu vor.");
  });
  return h(
    "div.notice",
    icon("eye-slash"),
    h("b", `${n} ${n === 1 ? "Seite" : "Seiten"} mit Organisatorischem`),
    h("span.muted", "Termine, Klausur, Kontakt oder Literatur fragt die App nicht ab."),
    btn ? h("div.actions", btn) : null,
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
function questionList(unitId, shared) {
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
    const add = h("button.btn.secondary", { type: "button", onclick: () => editQuestion({ unit_id: unitId, kind: null, shared, options: [], groups: [], spots: [] }, draw) }, icon("plus-circle"), "Aufgabe hinzufügen");
    if (!qs.length) return list.replaceChildren(h("p.muted", "Noch keine Fragen. Sie entstehen, sobald du die erste Lektion startest."), add);
    const hidden = qs.filter((q) => q.hidden).length;
    list.replaceChildren(
      h("p.muted", hidden ? `${qs.length - hidden} aktiv, ${hidden} ausgeblendet.` : "Unklar oder nicht prüfungsrelevant? Bearbeite oder blende die Frage aus."),
      add,
      ...qs.map((q) => {
        const toggle = h("button.btn.ghost", { type: "button" }, icon(q.hidden ? "eye" : "eye-slash"), q.hidden ? "Einblenden" : "Ausblenden");
        toggle.addEventListener("click", async () => {
          await call("hide_question", { questionId: q.id, hidden: !q.hidden }).catch(fail);
          draw();
        });
        const mark = h(
          "button.btn.ghost",
          { type: "button", "aria-pressed": String(q.marked), "aria-label": q.marked ? "Nicht mehr für die Klausur merken" : "Für die Klausur merken" },
          icon(q.marked ? "bookmark-fill" : "bookmark"),
          q.marked ? "Gemerkt" : "Merken",
        );
        mark.addEventListener("click", async () => {
          await call("mark_question", { questionId: q.id, marked: !q.marked }).catch(fail);
          draw();
        });
        return h(
          `div.q-item${q.hidden ? ".is-hidden" : ""}`,
          h("p.q-text", q.prompt),
          q.kind in KIND_LABEL && !["mc", "true_false", "fill_blank"].includes(q.kind) ? h("span.badge", KIND_LABEL[q.kind]) : null,
          h("p.muted", "Richtig: ", h("b", q.answer)),
          h("div.row", { style: { "flex-wrap": "wrap" } }, h("button.btn.ghost", { type: "button", onclick: () => editQuestion(q, draw) }, icon("pencil-simple"), "Bearbeiten"), toggle, mark),
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

const KIND_LABEL = {
  mc: "Multiple Choice",
  true_false: "Richtig oder falsch",
  fill_blank: "Lückentext",
  cloze: "Lückentext",
  sort: "Zuordnen / Matrix",
  order: "Reihenfolge",
  label: "Bild beschriften",
};
const NEW_KINDS = ["mc", "true_false", "cloze", "sort", "order", "label"];
const PROMPT_LABEL = { fill_blank: "Satz mit Lücke", cloze: "Text mit Lücken", true_false: "Aussage", mc: "Frage" };
const PROMPT_HINT = {
  fill_blank: "Markiere die Lücke mit drei Unterstrichen ___",
  cloze: "Setze jede Lücke in eckige Klammern: Die [SWOT]-Analyse verbindet [interne] und [externe] Sicht.",
};
const PLACEHOLDER = {
  sort: "z. B. Erstelle eine SWOT-Analyse für ein kleines Café in der Innenstadt.",
  order: "z. B. Bringe die Schritte der SWOT-Analyse in die richtige Reihenfolge.",
  label: "z. B. Beschrifte die Teile des Blattes.",
};
const SWOT_NAMES = ["Stärken", "Schwächen", "Chancen", "Risiken"];
const same = (a, b) => a.trim().toLowerCase() === b.trim().toLowerCase();
const lines = (s) => s.split("\n").map((x) => x.trim()).filter(Boolean);

/**
 * Formular-Sheet für eine Frage. Änderungen gelten für alle im Kurs.
 * Ohne `q.id` legt es eine neue Aufgabe in `q.unit_id` an; die Art wählt man oben.
 * `onSaved(frage)` bekommt die gespeicherte Fassung vom Kern (bereinigt).
 */
export function editQuestion(q, onSaved) {
  const isNew = !q.id;
  let kind = q.kind || "mc";
  const err = h("p.form-error", { id: "q-error", role: "alert", hidden: true });
  const prompt = h("textarea.input", { id: "q-prompt", rows: 3, value: q.prompt ?? "", "aria-describedby": "q-error" });
  const why = h("textarea.input", { id: "q-why", rows: 2, value: q.why ?? "", placeholder: "Ein Satz, warum die Antwort stimmt" });
  const promptLabel = h("label", { for: "q-prompt" });
  const promptHint = h("span.hint");
  const slot = h("div.stack");
  let fields;

  function setKind(k) {
    kind = k;
    fields = answerFields(k, q);
    slot.replaceChildren(fields.el);
    promptLabel.textContent = PROMPT_LABEL[k] ?? "Aufgabe";
    promptHint.textContent = PROMPT_HINT[k] ?? "";
    promptHint.hidden = !PROMPT_HINT[k];
    prompt.placeholder = PLACEHOLDER[k] ?? "";
    if (isNew && k === "label" && !prompt.value.trim()) prompt.value = "Beschrifte die Grafik.";
  }
  const picker = isNew
    ? h(
        "div.field",
        h("label", { for: "q-kind" }, "Art der Aufgabe"),
        h("select.input", { id: "q-kind", onchange: (e) => setKind(e.target.value) }, NEW_KINDS.map((k) => h("option", { value: k }, KIND_LABEL[k]))),
      )
    : null;
  setKind(kind);

  const save = h("button.btn.block", { type: "button" }, isNew ? "Aufgabe anlegen" : "Speichern");
  const s = openSheet({
    title: isNew ? "Neue Aufgabe" : "Frage bearbeiten",
    body: [
      h("p.muted", isNew ? (q.shared ? "Die Aufgabe gilt für alle in diesem Kurs." : "Sie kommt ab der nächsten Lektion vor.") : q.shared ? `${KIND_LABEL[kind]}. Die Änderung gilt für alle in diesem Kurs.` : KIND_LABEL[kind]),
      picker,
      h("div.field", promptLabel, prompt, promptHint),
      slot,
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
    const data = fields.read();
    const p = prompt.value.trim();
    // Schnelle Prüfung hier, die verbindliche macht der Kern.
    const problem =
      p.length < 8
        ? "Die Aufgabe braucht mindestens 8 Zeichen."
        : kind === "fill_blank" && !p.includes("__")
          ? "Im Satz fehlt die Lücke ___."
          : kind === "cloze" && !/\[[^\]]+\]/.test(p)
            ? "Setze mindestens eine Lücke in [eckige Klammern]."
            : kind === "mc" && data.options.length < 4
              ? "Multiple Choice braucht 4 Antworten."
              : kind === "fill_blank" && data.options.length < 2
                ? "Gib mindestens 2 Antworten zur Auswahl."
                : ["mc", "fill_blank"].includes(kind) && !data.answer
                  ? "Markiere die richtige Antwort."
                  : kind === "sort" && (data.groups.length < 2 || data.groups.reduce((n, g) => n + g.items.length, 0) < 3)
                    ? "Gib mindestens 2 Felder mit zusammen 3 Aussagen an."
                    : kind === "order" && data.options.length < 3
                      ? "Gib mindestens 3 Schritte an."
                      : kind === "label" && !data.image && !data.figureId
                        ? "Wähle zuerst ein Bild."
                        : kind === "label" && (!data.spots.length || data.spots.some((x) => !x.label.trim()))
                          ? "Tippe ins Bild, um Stellen zu setzen, und beschrifte jede."
                          : null;
    if (problem) return fail(problem);
    save.disabled = true;
    try {
      const body = { prompt: p, why: why.value, answer: "", options: [], ...data };
      const saved = isNew ? await call("create_question", { unitId: q.unit_id, kind, ...body }) : await call("update_question", { questionId: q.id, ...body });
      s.close();
      toast(q.shared ? "Gespeichert. Gilt für alle im Kurs." : "Gespeichert.");
      onSaved?.(saved);
    } catch (e) {
      fail(String(e));
      save.disabled = false;
    }
  });
}

/** Antwort-Teil des Formulars je Art. `read()` liefert die Felder für den Kern. */
function answerFields(kind, q) {
  const area = (id, label, value, opts = {}) => {
    const t = h("textarea.input", { id, rows: opts.rows ?? 4, value, placeholder: opts.placeholder ?? "" });
    return { t, el: h("div.field", h("label", { for: id }, label), opts.hint ? h("span.hint", opts.hint) : null, t) };
  };
  const sameKind = q.kind === kind;
  if (kind === "true_false") {
    let pick = sameKind ? q.answer : "Richtig";
    return {
      el: h("div.field", h("label", "Richtige Antwort"), segmented([["Richtig", "Richtig"], ["Falsch", "Falsch"]], pick, (v) => (pick = v), "Richtige Antwort")),
      read: () => ({ options: ["Richtig", "Falsch"], answer: pick }),
    };
  }
  if (kind === "mc" || kind === "fill_blank") {
    const rows = [0, 1, 2, 3].map((i) => {
      const o = (sameKind && q.options[i]) || "";
      const radio = h("input", { type: "radio", name: "q-correct", checked: o !== "" && same(o, q.answer), "aria-label": `Antwort ${i + 1} ist richtig` });
      const text = h("input.input", { value: o, autocomplete: "off", "aria-label": `Antwort ${i + 1}`, placeholder: kind === "fill_blank" && i > 1 ? "optional" : "" });
      return { radio, text, el: h("div.opt-row", radio, text) };
    });
    return {
      el: h("div.field", h("label", "Antworten"), h("span.hint", "Markiere die richtige."), ...rows.map((r) => r.el)),
      read: () => ({
        options: rows.filter((r) => r.text.value.trim()).map((r) => r.text.value.trim()),
        answer: rows.find((r) => r.radio.checked)?.text.value.trim() ?? "",
      }),
    };
  }
  if (kind === "cloze") {
    const a = area("q-spare", "Falsche Wörter zur Auswahl (optional)", sameKind ? q.options.join("\n") : "", { rows: 3, hint: "Eins pro Zeile. Die richtigen Wörter kommen aus den Klammern." });
    return { el: a.el, read: () => ({ options: lines(a.t.value) }) };
  }
  if (kind === "order") {
    const a = area("q-steps", "Schritte in der richtigen Reihenfolge", sameKind ? q.options.join("\n") : "", { rows: 6, hint: "Einer pro Zeile, ohne Nummern. Gemischt wird in der Lektion." });
    return { el: a.el, read: () => ({ options: lines(a.t.value) }) };
  }
  if (kind === "sort") {
    const groups = sameKind ? q.groups : [];
    const rows = [0, 1, 2, 3].map((i) => {
      const name = h("input.input", { value: groups[i]?.name ?? "", autocomplete: "off", "aria-label": `Name von Feld ${i + 1}`, placeholder: `z. B. ${SWOT_NAMES[i]}` });
      const items = h("textarea.input", { rows: 2, value: (groups[i]?.items ?? []).join("\n"), "aria-label": `Aussagen in Feld ${i + 1}`, placeholder: "Eine Aussage pro Zeile" });
      return { name, items, el: h("div.group-row", name, items) };
    });
    const spare = area("q-spare", "Passt nirgends (optional)", sameKind ? q.options.join("\n") : "", { rows: 2, hint: "Ablenker, eine pro Zeile. Sie bleiben in der Lektion unten liegen." });
    return {
      el: h("div.stack", h("div.field", h("label", "Felder"), h("span.hint", "2 bis 4 Felder. Vier Felder werden als Matrix gezeigt, „Stärken, Schwächen, Chancen, Risiken“ als SWOT."), ...rows.map((r) => r.el)), spare.el),
      read: () => ({
        groups: rows.filter((r) => r.name.value.trim()).map((r) => ({ name: r.name.value.trim(), items: lines(r.items.value) })),
        options: lines(spare.t.value),
      }),
    };
  }
  return figureEditor(q, sameKind);
}

/** Bild wählen (aus dem Skript oder eigenes Foto), Stellen antippen, verschieben, Größe ändern und beschriften. */
function figureEditor(q, sameKind) {
  let image = null; // neues Bild als data:-URL
  let figureId = sameKind ? q.figure_id ?? null : null;
  let spots = sameKind ? q.spots.map((x) => ({ ...x })) : [];
  const img = h("img", { alt: "Bild für die Aufgabe", draggable: "false" });
  const stage = h("div.figure.edit", img);
  const zoom = zoomable(stage, img, { maxH: 0.5 });
  zoom.el.hidden = true;
  const list = h("div.stack");
  const gallery = h("div.gallery");
  const file = h("input", { type: "file", accept: "image/*", hidden: true });
  const spare = h("textarea.input", { id: "q-spare", rows: 2, value: sameKind ? q.options.join("\n") : "", placeholder: "Eins pro Zeile" });
  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

  function show(src) {
    img.src = src;
    zoom.el.hidden = false;
    draw();
  }
  function setFigure(id, src) {
    figureId = id;
    image = null;
    spots = [];
    gallery.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(Number(b.dataset.id) === id)));
    show(src);
  }

  /** Stellen und Liste neu zeichnen. */
  function draw() {
    stage.querySelectorAll(".spot").forEach((e) => e.remove());
    spots.forEach((sp, i) => {
      const handle = h("span.handle", { "aria-hidden": "true" });
      const el = h(
        "div.spot.edit",
        { tabindex: "0", role: "group", "aria-label": `Stelle ${i + 1}: ${sp.label || "ohne Beschriftung"}. Pfeiltasten verschieben, mit Umschalt die Größe ändern.` },
        h("span.spot-num", String(i + 1)),
        h("span.spot-label", sp.label),
        handle,
      );
      const place = () => Object.assign(el.style, { left: `${sp.x * 100}%`, top: `${sp.y * 100}%`, width: `${sp.w * 100}%`, height: `${sp.h * 100}%` });
      place();
      drag(el, (dx, dy, start) => {
        sp.x = clamp(start.x + dx, 0, 1 - sp.w);
        sp.y = clamp(start.y + dy, 0, 1 - sp.h);
        place();
      }, () => ({ x: sp.x, y: sp.y }));
      drag(handle, (dx, dy, start) => {
        sp.w = clamp(start.w + dx, 0.03, 1 - sp.x);
        sp.h = clamp(start.h + dy, 0.02, 1 - sp.y);
        place();
      }, () => ({ w: sp.w, h: sp.h }));
      el.addEventListener("keydown", (e) => {
        const d = { ArrowLeft: [-0.01, 0], ArrowRight: [0.01, 0], ArrowUp: [0, -0.01], ArrowDown: [0, 0.01] }[e.key];
        if (!d) return;
        e.preventDefault();
        if (e.shiftKey) {
          sp.w = clamp(sp.w + d[0], 0.03, 1 - sp.x);
          sp.h = clamp(sp.h + d[1], 0.02, 1 - sp.y);
        } else {
          sp.x = clamp(sp.x + d[0], 0, 1 - sp.w);
          sp.y = clamp(sp.y + d[1], 0, 1 - sp.h);
        }
        place();
      });
      stage.append(el);
    });
    list.replaceChildren(
      ...spots.map((sp, i) => {
        const input = h("input.input", { value: sp.label, autocomplete: "off", "aria-label": `Beschriftung Stelle ${i + 1}`, placeholder: "Beschriftung" });
        input.addEventListener("input", () => {
          sp.label = input.value;
          stage.querySelectorAll(".spot-label")[i].textContent = input.value;
        });
        const del = h("button.icon-btn", { type: "button", "aria-label": `Stelle ${i + 1} löschen`, onclick: () => (spots.splice(i, 1), draw()) }, icon("trash"));
        return h("div.spot-row", h("span.n", String(i + 1)), input, del);
      }),
    );
  }

  /** Ziehen in Bild-Anteilen (0–1). Bewegung unter 4 px zählt als Antippen. */
  function drag(el, onMove, start) {
    el.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      e.preventDefault();
      el.setPointerCapture(e.pointerId);
      const r = stage.getBoundingClientRect();
      const s0 = start();
      const move = (ev) => onMove((ev.clientX - e.clientX) / r.width, (ev.clientY - e.clientY) / r.height, s0);
      const up = () => {
        el.removeEventListener("pointermove", move);
        el.removeEventListener("pointerup", up);
        el.removeEventListener("pointercancel", up);
      };
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", up);
      el.addEventListener("pointercancel", up);
    });
  }

  // Antippen ins Bild setzt eine neue Stelle an diese Position (click, damit Scrollen am Handy keine setzt).
  stage.addEventListener("click", (e) => {
    if (e.target !== img && e.target !== stage) return;
    const r = stage.getBoundingClientRect();
    const w = 0.22;
    const hh = 0.08;
    spots.push({ x: clamp((e.clientX - r.left) / r.width - w / 2, 0, 1 - w), y: clamp((e.clientY - r.top) / r.height - hh / 2, 0, 1 - hh), w, h: hh, label: "" });
    draw();
    list.querySelectorAll("input")[spots.length - 1]?.focus({ preventScroll: true });
  });

  // Eigenes Bild: auf höchstens 1600 px verkleinern, als JPEG speichern.
  file.addEventListener("change", async () => {
    const f = file.files[0];
    file.value = "";
    if (!f) return;
    try {
      const bmp = await createImageBitmap(f);
      const scale = Math.min(1, 1600 / Math.max(bmp.width, bmp.height));
      const c = h("canvas", { width: Math.round(bmp.width * scale), height: Math.round(bmp.height * scale) });
      const ctx = c.getContext("2d");
      ctx.fillStyle = "#fff";
      ctx.fillRect(0, 0, c.width, c.height);
      ctx.drawImage(bmp, 0, 0, c.width, c.height);
      let data = c.toDataURL("image/jpeg", 0.82);
      if (data.length > 1_900_000) data = c.toDataURL("image/jpeg", 0.6);
      setFigure(null, data);
      image = data;
    } catch {
      toast("Dieses Bild lässt sich nicht öffnen.");
    }
  });

  const loadGallery = async () => {
    gallery.replaceChildren(h("div.skeleton", { style: { height: "72px" } }));
    try {
      const figs = await call("unit_figures", { unitId: q.unit_id });
      if (!figs.length) return gallery.replaceChildren(h("p.muted", "In diesem Abschnitt wurden keine Grafiken gefunden. Nimm ein eigenes Bild, oder ersetze das PDF unter „Hinzufügen“, damit die Grafiken neu eingelesen werden."));
      gallery.replaceChildren(
        ...figs.map((f) =>
          h("button", { type: "button", "data-id": String(f.id), "aria-pressed": String(f.id === figureId), "aria-label": f.page ? `Grafik von Seite ${f.page}` : "Eigenes Bild", onclick: () => setFigure(f.id, f.image) }, h("img", { src: f.image, alt: "" })),
        ),
      );
    } catch (e) {
      gallery.replaceChildren(h("p.muted", String(e)));
    }
  };
  const source = segmented([["pdf", "Aus dem Skript"], ["own", "Eigenes Bild"]], "pdf", (v) => (v === "own" ? file.click() : loadGallery()), "Bildquelle");
  loadGallery();
  if (figureId) call("figure", { figureId }).then((f) => f && show(f.image)).catch(fail);

  return {
    el: h(
      "div.stack",
      h("div.field", h("label", "Bild"), source, file, gallery),
      zoom.el,
      h("p.hint", "Tippe ins Bild, um eine Stelle zu setzen. Ziehen verschiebt sie, der Punkt unten rechts ändert die Größe. Zum Vergrößern zwei Finger oder die Lupe."),
      list,
      h("div.field", h("label", { for: "q-spare" }, "Falsche Begriffe zur Auswahl (optional)"), spare),
    ),
    read: () => ({ spots: spots.map((x) => ({ ...x, label: x.label.trim() })), options: lines(spare.value), ...(image ? { image } : { figureId }) }),
  };
}
