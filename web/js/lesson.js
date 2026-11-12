// Lektion: eine Aufgabe nach der anderen, Prüfen ohne KI, falsche Aufgaben kommen am Ende erneut.
import { call } from "./api.js";
import { confetti, confirmSheet, countUp, h, icon, openSheet, reduceMotion, shake, sound, toast } from "./ui.js";
import { emit, state, subscribe } from "./state.js";
import { editQuestion, openChat } from "./unit.js";

const LABEL = {
  mc: "Wähle die richtige Antwort",
  true_false: "Stimmt das?",
  fill_blank: "Fülle die Lücke",
  match: "Ordne die Begriffe zu",
};
const PRAISE = ["Richtig!", "Genau so!", "Sitzt!", "Stark!", "Sehr gut!"];
const norm = (s) => s.trim().toLowerCase();

function shuffle(a) {
  const b = [...a];
  for (let i = b.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [b[i], b[j]] = [b[j], b[i]];
  }
  return b;
}

export function openLesson(unitId, onExit) {
  const fill = h("i");
  const progress = h("div.progress", { role: "progressbar", "aria-label": "Fortschritt der Lektion", "aria-valuemin": "0", "aria-valuemax": "100", "aria-valuenow": "0" }, fill);
  const combo = h("span.combo.hide", { "aria-live": "polite" });
  const closeBtn = h("button.icon-btn", { type: "button", "aria-label": "Lektion beenden" }, icon("x"));
  const menuBtn = h("button.icon-btn", { type: "button", "aria-label": "Diese Frage bearbeiten oder ausblenden", hidden: true }, icon("dots-three"));
  const body = h("div.lesson-body", { tabindex: "-1" });
  const mainBtn = h("button.btn", { type: "button", disabled: true }, "Prüfen");
  const foot = h("div.lesson-foot", h("div.inner", mainBtn));
  const fb = h("div.feedback", { "aria-live": "assertive" });
  const root = h("div.lesson", { role: "dialog", "aria-modal": "true", "aria-label": unitId ? "Lektion" : "Wiederholung" }, h("div.lesson-top", closeBtn, progress, combo, menuBtn), body, foot, fb);
  document.body.append(root);
  document.body.style.overflow = "hidden";

  let queue = [];
  let total = 0;
  let solved = 0;
  let firstRight = 0;
  let run = 0;
  let cur = null;
  let lastOk = false;
  let phase = "loading"; // answer | checking | feedback | done
  let unsub = null;
  const t0 = Date.now();
  const streakBefore = state.stats?.streak ?? 0;

  const setReady = (ok) => (mainBtn.disabled = !ok);

  // ---------- Aufgaben-Ansichten ----------
  function choice(ex, big) {
    let sel = null;
    const tiles = ex.options.map((o, i) =>
      h(
        `button.tile${big ? ".big" : ""}`,
        { type: "button", "aria-pressed": "false", onclick: () => pick(i) },
        big ? icon(o === "Richtig" ? "check-circle-fill" : "x-circle-fill") : h("span.key", { "aria-hidden": "true" }, String(i + 1)),
        h("span", o),
      ),
    );
    if (big) {
      tiles[0].querySelector(".icon").style.color = "var(--right)";
      tiles[1].querySelector(".icon").style.color = "var(--wrong)";
    }
    function pick(i) {
      if (phase !== "answer") return;
      sel = i;
      tiles.forEach((t, j) => t.setAttribute("aria-pressed", String(i === j)));
      setReady(true);
    }
    return {
      el: h(`div.options${big ? ".two" : ""}`, tiles),
      key: (n) => n <= tiles.length && pick(n - 1),
      value: () => (sel == null ? null : ex.options[sel]),
      mark(ok, expected) {
        tiles.forEach((t) => (t.disabled = true));
        const t = tiles[sel];
        t.setAttribute("aria-pressed", "false");
        t.classList.add(ok ? "is-right" : "is-wrong");
        if (!ok) {
          shake(t);
          tiles.find((_, j) => norm(ex.options[j]) === norm(expected))?.classList.add("is-right");
        }
      },
    };
  }

  function blank(ex) {
    const [pre, ...rest] = ex.prompt.split("___");
    let chosen = null;
    const gap = h("button.blank", { type: "button", "aria-label": "Lücke, antippen zum Leeren" });
    const chips = ex.options.map((o, i) => h("button.tile", { type: "button", onclick: () => pick(i) }, o));
    function pick(i) {
      if (phase !== "answer") return;
      if (chosen != null) chips[chosen].classList.remove("used");
      chosen = i;
      chips[i].classList.add("used");
      gap.textContent = ex.options[i];
      gap.classList.remove("filled");
      void gap.offsetWidth; // Animation neu starten
      gap.classList.add("filled");
      setReady(true);
    }
    gap.addEventListener("click", () => {
      if (phase !== "answer" || chosen == null) return;
      chips[chosen].classList.remove("used");
      chosen = null;
      gap.textContent = "";
      gap.classList.remove("filled");
      setReady(false);
    });
    return {
      el: h("div.stack", h("p.blank-sentence", pre, gap, rest.join("___")), h("div.bank", chips)),
      key: (n) => n <= chips.length && pick(n - 1),
      value: () => (chosen == null ? null : ex.options[chosen]),
      mark(ok) {
        chips.forEach((c) => (c.disabled = true));
        gap.disabled = true;
        gap.classList.add(ok ? "is-right" : "is-wrong");
        if (!ok) shake(gap);
      },
    };
  }

  function match(ex) {
    let left = null;
    let right = null;
    let done = 0;
    let mistakes = 0;
    const mk = (side, item) => {
      const t = h(`button.tile${side === "R" ? ".def" : ""}`, { type: "button", "aria-pressed": "false" }, item.text);
      t.addEventListener("click", () => choose(side, item.i, t));
      return t;
    };
    const L = shuffle(ex.pairs.map((p, i) => ({ i, text: p[0] }))).map((x) => mk("L", x));
    const R = shuffle(ex.pairs.map((p, i) => ({ i, text: p[1] }))).map((x) => mk("R", x));
    function choose(side, i, t) {
      if (phase !== "answer" || t.disabled) return;
      const list = side === "L" ? L : R;
      list.forEach((x) => x.setAttribute("aria-pressed", "false"));
      t.setAttribute("aria-pressed", "true");
      if (side === "L") left = { i, t };
      else right = { i, t };
      if (!left || !right) return;
      const a = left;
      const b = right;
      left = right = null;
      [a.t, b.t].forEach((x) => x.setAttribute("aria-pressed", "false"));
      if (a.i === b.i) {
        a.t.classList.add("is-right");
        b.t.classList.add("is-right");
        a.t.disabled = b.t.disabled = true;
        sound.right();
        setTimeout(() => [a.t, b.t].forEach((x) => (x.classList.remove("is-right"), x.classList.add("is-done"))), 350);
        if (++done === ex.pairs.length) setTimeout(() => verdict(mistakes <= 1, null, mistakes <= 1 ? null : "Schau dir die Begriffe auf der Lernkarte noch einmal an."), 450);
      } else {
        mistakes++;
        sound.wrong();
        [a.t, b.t].forEach((x) => {
          x.classList.add("is-wrong");
          shake(x);
          setTimeout(() => x.classList.remove("is-wrong"), 450);
        });
      }
    }
    return { el: h("div.match", h("div.col", L), h("div.col", R)), value: () => null, mark() {} };
  }

  // ---------- Ablauf ----------
  function show() {
    const ex = cur.ex;
    phase = "answer";
    setReady(false);
    mainBtn.textContent = "Prüfen";
    foot.hidden = ex.kind === "match";
    menuBtn.hidden = !ex.id; // „Zuordnen“ kommt aus den Begriffen, nicht aus einer Frage
    cur.view = ex.kind === "mc" ? choice(ex) : ex.kind === "true_false" ? choice(ex, true) : ex.kind === "fill_blank" ? blank(ex) : match(ex);
    body.replaceChildren(
      ...[
        h("p.q-kind", ex.retry ? `Nochmal: ${LABEL[ex.kind]}` : LABEL[ex.kind]),
        ex.kind === "fill_blank" || ex.kind === "match" ? null : h("h1.q-prompt", ex.prompt),
        cur.view.el,
      ].filter(Boolean),
    );
    body.scrollTop = 0;
    body.focus({ preventScroll: true }); // Fokus weg vom geschlossenen Feedback-Button
    if (!reduceMotion()) body.animate([{ opacity: 0, transform: "translateX(14px)" }, { opacity: 1, transform: "none" }], { duration: 200, easing: "cubic-bezier(0.23, 1, 0.32, 1)" });
  }

  function updateProgress() {
    const p = total ? solved / total : 0;
    fill.style.setProperty("--p", String(p));
    progress.setAttribute("aria-valuenow", String(Math.round(p * 100)));
    combo.replaceChildren(icon("fire-fill"), h("span.num", `${run} in Folge`));
    combo.classList.toggle("hide", run < 3);
  }

  async function check() {
    if (phase !== "answer") return;
    const given = cur.view.value();
    if (given == null) return;
    phase = "checking";
    setReady(false);
    try {
      const res = await call("answer", { questionId: cur.ex.id, given, retry: !!cur.ex.retry });
      cur.view.mark(res.correct, res.expected);
      verdict(res.correct, res.expected, res.why);
    } catch (e) {
      phase = "answer";
      setReady(true);
      toast(String(e));
    }
  }

  function verdict(ok, expected, why) {
    phase = "feedback";
    lastOk = ok;
    if (ok) {
      if (!cur.ex.retry) firstRight++;
      solved++;
      run++;
      if (cur.ex.kind !== "match") sound.right();
    } else {
      run = 0;
      if (cur.ex.kind !== "match") sound.wrong();
      queue.push({ ...cur.ex, retry: true });
    }
    updateProgress();

    const nextBtn = h(`button.btn.${ok ? "right" : "wrong"}`, { type: "button", onclick: next }, "Weiter");
    const ex = cur.ex;
    const whyBtn =
      !ok && ex.id
        ? h("button.btn.secondary", {
            type: "button",
            onclick: () =>
              openChat(unitId, "", {
                shown: "Warum ist das so?",
                sent: `Frage: ${ex.prompt}\nRichtige Antwort: ${expected}\nMeine Antwort: ${cur.view.value()}\nErkläre mir in 3-4 einfachen Sätzen, warum die richtige Antwort stimmt.`,
              }),
          }, icon("sparkle"), "Warum?")
        : null;
    fb.className = `feedback${ok ? "" : " wrong"}`;
    fb.replaceChildren(
      h(
        "div.inner",
        h("h2", icon(ok ? "check-circle-fill" : "x-circle-fill"), ok ? PRAISE[Math.floor(Math.random() * PRAISE.length)] : "Nicht ganz"),
        !ok && expected ? h("p.detail", "Richtig ist: ", h("b", expected)) : null,
        why ? h("p.why", why) : null,
        h("div.actions", whyBtn, nextBtn),
      ),
    );
    requestAnimationFrame(() => {
      fb.classList.add("open");
      nextBtn.focus({ preventScroll: true });
    });
  }

  function next() {
    if (phase !== "feedback") return;
    advance();
  }

  function advance() {
    fb.classList.remove("open");
    if (!queue.length) return total ? finish() : exit();
    cur = { ex: queue.shift() };
    show();
  }

  // ---------- Frage bearbeiten / ausblenden ----------
  async function questionMenu() {
    const ex = cur?.ex;
    if (!ex?.id || (phase !== "answer" && phase !== "feedback")) return;
    let q;
    try {
      q = await call("question", { questionId: ex.id });
    } catch (e) {
      return toast(String(e));
    }
    const edit = h("button.btn.secondary.block", { type: "button" }, icon("pencil-simple"), "Frage bearbeiten");
    const hide = h("button.btn.secondary.block", { type: "button" }, icon("eye-slash"), "Frage ausblenden");
    const del = q.mine && q.shared ? h("button.btn.ghost", { type: "button", style: { color: "var(--wrong)" } }, icon("trash"), "Für alle löschen") : null;
    const s = openSheet({
      title: "Diese Frage",
      body: [h("p.muted", "Unklar formuliert oder nicht prüfungsrelevant?"), edit, hide, h("p.hint", "Ausgeblendet kommt sie bei dir in keiner Lektion und Wiederholung mehr vor."), del],
    });
    edit.addEventListener("click", () => {
      s.close();
      editQuestion(q, (saved) => {
        // Aktuelle Aufgabe und spätere Wiederholungen derselben Frage übernehmen die neue Fassung.
        for (const e of [cur.ex, ...queue]) {
          if (e.id === saved.id) Object.assign(e, { prompt: saved.prompt, options: e.kind === "true_false" ? saved.options : shuffle(saved.options) });
        }
        if (phase === "answer") show();
      });
    });
    hide.addEventListener("click", async () => {
      s.close();
      try {
        await call("hide_question", { questionId: ex.id, hidden: true });
      } catch (e) {
        return toast(String(e));
      }
      drop(ex);
      toast("Ausgeblendet.", { action: "Rückgängig", onAction: () => call("hide_question", { questionId: ex.id, hidden: false }).catch((e) => toast(String(e))) });
    });
    del?.addEventListener("click", async () => {
      s.close();
      const yes = await confirmSheet({
        title: "Für alle löschen?",
        text: "Die Frage verschwindet für alle in diesem Kurs, auch ihr Fortschritt. Das lässt sich nicht rückgängig machen.",
        confirm: "Endgültig löschen",
        danger: true,
      });
      if (!yes) return;
      try {
        await call("delete_question", { questionId: ex.id });
      } catch (e) {
        return toast(String(e));
      }
      drop(ex);
      toast("Gelöscht.");
    });
  }

  /** Frage aus dieser Lektion nehmen: nie gelöst → zählt nicht mehr zur Gesamtzahl. */
  function drop(ex) {
    if (cur?.ex.id !== ex.id) return;
    queue = queue.filter((e) => e.id !== ex.id);
    if (!(phase === "feedback" && lastOk)) total = Math.max(0, total - 1);
    updateProgress();
    advance();
  }

  async function finish() {
    phase = "done";
    menuBtn.hidden = true;
    let res;
    try {
      res = await call("finish_lesson", { unitId, correct: firstRight, total });
    } catch (e) {
      toast(String(e));
      return exit();
    }
    state.stats = res.stats;
    emit("stats", res.stats);
    const s = res.stats;
    const acc = Math.round((firstRight / Math.max(1, total)) * 100);
    const secs = Math.round((Date.now() - t0) / 1000);
    const grew = s.streak > streakBefore;
    const goalNow = s.xp_today >= s.goal && s.xp_today - res.xp < s.goal;
    const xpEl = h("b", "0");
    const streakText = grew ? (s.streak === 1 ? "Dein Streak beginnt heute." : `${s.streak}. Tag in Folge. Weiter so!`) : `Streak: ${s.streak} ${s.streak === 1 ? "Tag" : "Tage"}`;
    foot.hidden = false;
    combo.classList.add("hide");
    body.replaceChildren(
      h(
        "div.done-screen",
        icon("fire-fill", `flame${grew ? " ignite" : ""}`),
        h("h1", unitId ? "Lektion geschafft!" : "Wiederholung geschafft!"),
        h("p.muted", streakText),
        goalNow ? h("p", { style: { "font-weight": "700" } }, icon("target"), " Tagesziel erreicht") : null,
        h(
          "div.tally",
          h("div.xp", xpEl, h("span", "XP")),
          h("div", h("b", `${acc} %`), h("span", "Treffer")),
          h("div", h("b", `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`), h("span", "Zeit")),
        ),
      ),
    );
    countUp(xpEl, res.xp);
    sound.done();
    if (acc >= 80 || goalNow) confetti();
    mainBtn.textContent = "Weiter";
    setReady(true);
    mainBtn.focus({ preventScroll: true });
  }

  // ---------- Laden & Warten ----------
  async function load() {
    phase = "loading";
    foot.hidden = true;
    body.replaceChildren(h("div.stack", h("div.skeleton", { style: { width: "40%" } }), h("div.skeleton", { style: { height: "36px" } }), h("div.skeleton", { style: { height: "62px" } }), h("div.skeleton", { style: { height: "62px" } })));
    let res;
    try {
      res = await call("start_lesson", { unitId });
    } catch (e) {
      return message("warning", "Die Lektion konnte nicht starten", String(e));
    }
    if (res.failed) {
      return message("warning", "Fragen konnten nicht erstellt werden", "Die KI hat für diesen Abschnitt keine brauchbaren Fragen geliefert.", [
        h("button.btn", { type: "button", onclick: () => call("retry_questions", { unitId }).then(wait) }, icon("arrow-clockwise"), "Neu erstellen"),
      ]);
    }
    if (!res.ready) return wait();
    if (!res.exercises.length) return message("seal-check-fill", "Gerade ist nichts fällig", "Fragen kommen zur Wiederholung zurück, kurz bevor du sie vergessen würdest.");
    queue = res.exercises;
    total = queue.length;
    updateProgress();
    cur = { ex: queue.shift() };
    show();
  }

  function wait() {
    const ready = state.ai?.state === "ready";
    message("sparkle", "Fragen werden erstellt", ready ? "Die KI liest diesen Abschnitt und schreibt Übungsfragen. Das dauert meist unter einer Minute." : "Sobald die KI eingerichtet ist, entstehen hier die Fragen.");
    unsub?.();
    unsub = subscribe((kind) => {
      if (kind === "course-updated" || kind === "ai") {
        unsub();
        unsub = null;
        load();
      }
    });
  }

  function message(ic, title, text, actions = []) {
    foot.hidden = true;
    menuBtn.hidden = true;
    body.replaceChildren(h("div.empty", h("div.art", icon(ic)), h("h1", title), h("p", text), ...actions, h("button.btn.secondary", { type: "button", onclick: exit }, "Zurück")));
  }

  // ---------- Steuerung ----------
  mainBtn.addEventListener("click", () => (phase === "done" ? exit() : check()));
  closeBtn.addEventListener("click", askClose);
  menuBtn.addEventListener("click", questionMenu);

  async function askClose() {
    if (phase === "done" || phase === "loading" || total === 0) return exit();
    const leave = await confirmSheet({
      title: "Lektion beenden?",
      text: "Deine Antworten sind gespeichert. XP gibt es aber erst, wenn du die Lektion abschließt.",
      confirm: "Beenden",
      cancel: "Weiterlernen",
      danger: true,
    });
    if (leave) exit();
  }

  function onKey(e) {
    if (document.querySelector(".sheet")) return; // Chat oder Rückfrage offen
    if (e.key === "Escape") {
      e.preventDefault();
      askClose();
    } else if (e.key === "Enter") {
      if (e.target instanceof Element && e.target.closest(".feedback, .lesson-top, input, textarea")) return; // dort reagiert das Element selbst
      e.preventDefault();
      if (phase === "answer") check();
      else if (phase === "feedback") next();
      else if (phase === "done") exit();
    } else if (/^[1-9]$/.test(e.key) && phase === "answer") {
      cur.view.key?.(Number(e.key));
    }
  }
  document.addEventListener("keydown", onKey);

  function close() {
    unsub?.();
    document.removeEventListener("keydown", onKey);
    document.body.style.overflow = "";
    root.remove();
  }
  function exit() {
    close();
    onExit();
  }

  load();
  return { close };
}
