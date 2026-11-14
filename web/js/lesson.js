// Lektion: eine Aufgabe nach der anderen, Prüfen ohne KI, falsche Aufgaben kommen am Ende erneut.
import { call } from "./api.js";
import { confetti, confirmSheet, countUp, flip, h, icon, openSheet, reduceMotion, shake, sound, toast, zoomable } from "./ui.js";
import { emit, state, subscribe } from "./state.js";
import { editQuestion, openChat } from "./unit.js";
import { fail, openReport } from "./report.js";

const LABEL = {
  mc: "Wähle die richtige Antwort",
  true_false: "Stimmt das?",
  fill_blank: "Fülle die Lücke",
  cloze: "Fülle die Lücken",
  sort: "Ordne zu",
  order: "Reihenfolge",
  label: "Grafik beschriften",
  match: "Ordne die Begriffe zu",
};
const CLASSIC = ["mc", "true_false", "fill_blank"];
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

/**
 * Lektion einer Einheit (`unitId`), „Falsche wiederholen“ (`wrongCourse`), Prüfungstraining (`examCourse`) eines Kurses
 * oder die tägliche Wiederholung (alles leer). `battleId`: Live-Battle, geprüft und gestoppt vom Server.
 */
export function openLesson({ unitId = null, wrongCourse = null, examCourse = null, battleId = null }, onExit) {
  const fill = h("i");
  const progress = h("div.progress", { role: "progressbar", "aria-label": "Fortschritt der Lektion", "aria-valuemin": "0", "aria-valuemax": "100", "aria-valuenow": "0" }, fill);
  const combo = h("span.combo.hide", { "aria-live": "polite" });
  const rival = h("span.chip.rival", { hidden: true });
  const closeBtn = h("button.icon-btn", { type: "button", "aria-label": "Lektion beenden" }, icon("x"));
  const menuBtn = h("button.icon-btn", { type: "button", "aria-label": "Diese Frage merken, bearbeiten, ausblenden oder melden", hidden: true }, icon("dots-three"));
  const body = h("div.lesson-body", { tabindex: "-1" });
  const mainBtn = h("button.btn", { type: "button", disabled: true }, "Prüfen");
  const foot = h("div.lesson-foot", h("div.inner", mainBtn));
  const fb = h("div.feedback", { "aria-live": "assertive" });
  const root = h("div.lesson", { role: "dialog", "aria-modal": "true", "aria-label": battleId ? "Battle" : unitId ? "Lektion" : examCourse ? "Prüfungstraining" : "Wiederholung" }, h("div.lesson-top", closeBtn, progress, combo, rival, menuBtn), body, foot, fb);
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
  let battle = null; // letzter Stand vom Server
  let poll = null;
  let countTimer = null;
  let resultShown = false;
  let ticking = false;
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

  // ---------- Ablegen: Begriffe in Felder, Kette, Lücken oder Bildstellen ----------
  /**
   * Gemeinsame Mechanik für die Strukturaufgaben. Antippen (Begriff → Zone oder Zone → Begriff) oder Ziehen.
   * zones: [{ el, list, name, cap }] – `list` nimmt die Begriffe auf, cap 1 = Lücke/Stelle (tauscht), sonst beliebig viele.
   * mode "fill": Antippen eines Begriffs füllt die gewählte bzw. nächste freie Zone (Lücken, Kette).
   */
  function placer({ options, zones, bank, mode = "pick", ready }) {
    const live = h("p.visually-hidden", { "aria-live": "polite" });
    const chips = options.map((text, i) => h("button.piece", { type: "button", "aria-pressed": "false", "data-i": String(i) }, text));
    const at = new Map(chips.map((c) => [c, null])); // Begriff → Zonen-Index, null = Auswahl unten
    let sel = null;
    let selZone = null;
    let dragged = false;
    bank.append(...chips);

    // Reihenfolge wie auf dem Bildschirm (wichtig für die Kette)
    const inZone = (zi) => [...(zi == null ? bank : zones[zi].list).children].filter((c) => chips.includes(c));
    const markSel = () => {
      chips.forEach((c) => c.setAttribute("aria-pressed", String(c === sel)));
      zones.forEach((z, zi) => z.el.classList.toggle("is-picked", zi === selZone));
    };
    // Kleine Auswahl klebt unten am Bildschirm (Felder scrollen darüber); eine große würde alles verdecken.
    const dock = () => bank.classList.toggle("dock", bank.offsetHeight <= body.clientHeight * 0.4);
    const changed = () => {
      setReady(ready(zones.map((_, zi) => inZone(zi).length), inZone(null).length));
      dock();
    };
    requestAnimationFrame(dock);

    /** Begriff an seinen Platz: Zone zi oder zurück in die Auswahl (an seine alte Stelle). */
    function move(c, zi) {
      const zone = zones[zi];
      const bumped = zone?.cap === 1 ? inZone(zi).find((x) => x !== c) : null;
      flip(chips, () => {
        for (const x of [bumped, zi == null ? c : null].filter(Boolean)) {
          at.set(x, null);
          const next = chips.find((y) => at.get(y) === null && y !== x && Number(y.dataset.i) > Number(x.dataset.i));
          bank.insertBefore(x, next ?? null);
        }
        if (zone) {
          at.set(c, zi);
          zone.list.append(c);
        }
        if (c.style.transform) c.style.transform = "";
      });
      sel = null;
      selZone = null;
      markSel();
      live.textContent = zone ? `${c.textContent} → ${zone.name}` : `${c.textContent} zurück in die Auswahl`;
      changed();
    }

    function tapChip(c) {
      if (phase !== "answer") return;
      const zi = at.get(c);
      if (zi != null && (mode === "fill" || zones[zi].cap === 1)) return move(c, null); // aus Lücke/Kette/Stelle zurück
      if (zi == null && selZone != null) return move(c, selZone);
      if (zi == null && mode === "fill") {
        const free = zones.findIndex((z, i) => inZone(i).length < z.cap);
        if (free >= 0) return move(c, free);
      }
      sel = sel === c ? null : c;
      selZone = null;
      markSel();
    }

    function tapZone(zi) {
      if (phase !== "answer") return;
      if (sel) return move(sel, zi);
      if (zones[zi].cap === 1) {
        selZone = selZone === zi ? null : zi;
        markSel();
      }
    }

    chips.forEach((c) => c.addEventListener("click", () => (dragged ? (dragged = false) : tapChip(c))));
    zones.forEach((z, zi) => z.el.addEventListener("click", (e) => !e.target.closest(".piece") && tapZone(zi)));
    bank.addEventListener("click", (e) => !e.target.closest(".piece") && sel && at.get(sel) != null && move(sel, null));

    // Ziehen: folgt dem Finger 1:1, die Zone darunter leuchtet, Loslassen setzt den Begriff per FLIP ab.
    const hit = (x, y) => {
      const inside = (el) => {
        const r = el.getBoundingClientRect();
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      };
      // Im gezoomten Bild zählt eine Stelle nur, wenn sie gerade zu sehen ist.
      const visible = (el) => {
        const clip = el.closest(".figure-wrap.zoomed");
        return !clip || inside(clip);
      };
      if (inside(bank)) return "bank"; // die Auswahl klebt unten und liegt dann über den Feldern
      const zi = zones.findIndex((z) => inside(z.el) && visible(z.el));
      return zi >= 0 ? zi : -1;
    };
    chips.forEach((c) =>
      c.addEventListener("pointerdown", (e) => {
        if (phase !== "answer" || e.button > 0) return;
        c.setPointerCapture(e.pointerId);
        const x0 = e.clientX;
        const y0 = e.clientY;
        let over = -1;
        let on = false;
        const moveH = (ev) => {
          const dx = ev.clientX - x0;
          const dy = ev.clientY - y0;
          if (!on && Math.hypot(dx, dy) < 6) return;
          if (!on) {
            on = true;
            c.getAnimations().forEach((a) => a.cancel());
            c.classList.add("is-dragging");
          }
          c.style.transform = `translate(${dx}px, ${dy}px) scale(1.04)`;
          const now = hit(ev.clientX, ev.clientY);
          if (now !== over) {
            zones[over]?.el.classList.remove("is-over");
            zones[now]?.el.classList.add("is-over");
            over = now;
          }
        };
        const up = (ev) => {
          c.removeEventListener("pointermove", moveH);
          c.removeEventListener("pointerup", up);
          c.removeEventListener("pointercancel", up);
          if (!on) return;
          dragged = true; // folgenden Klick schlucken
          setTimeout(() => (dragged = false), 0);
          c.classList.remove("is-dragging");
          zones[over]?.el.classList.remove("is-over");
          const target = ev.type === "pointercancel" ? -1 : hit(ev.clientX, ev.clientY);
          if (target === "bank") move(c, null);
          else if (target >= 0) move(c, target);
          else flip([c], () => (c.style.transform = ""), 260); // zurückgleiten
        };
        c.addEventListener("pointermove", moveH);
        c.addEventListener("pointerup", up);
        c.addEventListener("pointercancel", up);
      }),
    );

    return {
      chips,
      live,
      inZone,
      /** Zifferntaste: gewählten Begriff in Zone n legen, sonst Begriff n wählen. */
      key(n) {
        if (sel) return n <= zones.length && move(sel, n - 1);
        const free = inZone(null)[n - 1];
        if (free) tapChip(free);
      },
      /** Lösung zeigen: plan(c) → Ziel-Zone oder null. Richtig liegende bleiben, der Rest gleitet gestaffelt an seinen Platz. */
      settle(plan, order) {
        chips.forEach((c) => (c.disabled = true));
        const wrong = chips.filter((c) => at.get(c) !== plan(c) || (order && at.get(c) != null && order(c) !== inZone(at.get(c)).indexOf(c)));
        chips.forEach((c) => at.get(c) != null && c.classList.add(wrong.includes(c) ? "is-wrong" : "is-right"));
        wrong.filter((c) => at.get(c) != null).forEach(shake);
        if (!wrong.length) return;
        setTimeout(() => {
          flip(chips, () => {
            for (const c of [...chips].sort((a, b) => (order ? order(a) - order(b) : 0))) {
              const zi = plan(c);
              const was = at.get(c);
              at.set(c, zi);
              (zi == null ? bank : zones[zi].list).append(c);
              c.classList.remove("is-wrong");
              if (zi != null && (was !== zi || wrong.includes(c))) c.classList.add("is-fixed");
            }
          }, 380);
        }, reduceMotion() ? 0 : 550);
      },
    };
  }

  /** Ordnet Begriffe den Zielen zu, auch wenn ein Wort mehrfach vorkommt (z. B. zweimal „Blatt“). */
  function assign(chips, wants) {
    const free = new Set(chips);
    const plan = new Map();
    wants.forEach((w, zi) => {
      const c = [...free].find((x) => norm(x.textContent) === norm(w));
      if (c) {
        free.delete(c);
        plan.set(c, zi);
      }
    });
    return (c) => plan.get(c) ?? null;
  }

  /** Lückentext mit 1–5 Lücken. `fill_blank` (eine Lücke) schickt wie bisher nur das Wort. */
  function blank(ex) {
    const parts = ex.prompt.split("___");
    const zones = parts.slice(1).map((_, i) => {
      const gap = h("span.blank", { role: "group", "aria-label": `Lücke ${i + 1}` });
      return { el: gap, list: gap, name: `Lücke ${i + 1}`, cap: 1 };
    });
    const bank = h("div.bank");
    const sentence = h("p.blank-sentence", parts.flatMap((p, i) => (i ? [zones[i - 1].el, p] : [p])));
    const pl = placer({ options: ex.options, zones, bank, mode: "fill", ready: (n) => n.every((x) => x === 1) });
    const words = () => zones.map((_, i) => pl.inZone(i)[0]?.textContent ?? "");
    return {
      el: h("div.stack", sentence, bank, pl.live),
      key: pl.key,
      value: () => (ex.kind === "fill_blank" ? words()[0] || null : words()),
      text: () => words().join(" · "),
      mark(ok, expected, solution) {
        pl.settle(assign(pl.chips, solution ?? [expected]));
      },
    };
  }

  const SWOT = [/^(stärke|strength)/i, /^(schwäche|weakness)/i, /^(chance|opportunit)/i, /^(risik|threat|gefahr|bedrohung)/i];

  /** Matrix oder Felder. Vier SWOT-Felder bekommen die Achsen intern/extern und hilfreich/schädlich. */
  function board(ex) {
    const slot = ex.groups.map((g) => SWOT.findIndex((re) => re.test(g.trim())));
    const swot = ex.groups.length === 4 && new Set(slot).size === 4 && !slot.includes(-1);
    // ponytail: nur SWOT hat Achsen; weitere Raster (BCG, Ansoff) bei Bedarf hier ergänzen.
    const order = swot ? [0, 1, 2, 3].map((s) => slot.indexOf(s)) : ex.groups.map((_, i) => i);
    const zones = order.map((gi, pos) => {
      const list = h("div.zone-list");
      const el = h("div.zone", { "data-tone": String(pos % 4) }, h("button.zone-head", { type: "button", "aria-label": `In „${ex.groups[gi]}“ legen` }, h("span.num", String(pos + 1)), ex.groups[gi]), list);
      return { el, list, name: ex.groups[gi], cap: Infinity };
    });
    const bank = h("div.bank");
    const pl = placer({ options: ex.options, zones, bank, ready: (n) => n.every((x) => x > 0) });
    const cols = ex.groups.length === 4 ? 2 : ex.groups.length;
    const grid = swot
      ? h("div.board.matrix.swot", h("span"), h("span.axis", "hilfreich"), h("span.axis", "schädlich"), h("span.axis.side", "intern"), zones[0].el, zones[1].el, h("span.axis.side", "extern"), zones[2].el, zones[3].el)
      : h("div.board", { style: { "--cols": String(cols) } }, zones.map((z) => z.el));
    return {
      el: h("div.stack", grid, h("p.hint", "Was nirgends passt, bleibt unten."), bank, pl.live),
      key: pl.key,
      value: () => Object.fromEntries(zones.map((z, zi) => [z.name, pl.inZone(zi).map((c) => c.textContent)])),
      text: () => zones.map((z, zi) => `${z.name}: ${pl.inZone(zi).map((c) => c.textContent).join(", ") || "–"}`).join("; "),
      mark(ok, expected, solution) {
        const home = new Map((solution ?? []).flatMap((g) => g.items.map((i) => [norm(i), zones.findIndex((z) => z.name === g.name)])));
        pl.settle((c) => home.get(norm(c.textContent)) ?? null);
      },
    };
  }

  /** Reihenfolge als Prozesskette: Antippen hängt den Schritt an, Antippen in der Kette legt ihn zurück. */
  function chain(ex) {
    const list = h("div.chain", { role: "group", "aria-label": "Deine Reihenfolge" });
    const zones = [{ el: list, list, name: "Kette", cap: ex.options.length }];
    const bank = h("div.bank");
    const pl = placer({ options: ex.options, zones, bank, mode: "fill", ready: (_, rest) => rest === 0 });
    const steps = () => pl.inZone(0).map((c) => c.textContent);
    return {
      el: h("div.stack", list, bank, pl.live),
      key: pl.key,
      value: () => steps(),
      text: () => steps().join(" → "),
      mark(ok, expected, solution) {
        const rank = new Map((solution ?? []).map((s, i) => [norm(s), i]));
        pl.settle(() => 0, (c) => rank.get(norm(c.textContent)) ?? 99);
      },
    };
  }

  /** Grafik beschriften: nummerierte Stellen über dem Bild (zoombar), Begriffe darunter. */
  function figure(ex) {
    const img = h("img", { alt: "Grafik zum Beschriften", draggable: "false" });
    const stage = h("div.figure", img);
    const zones = ex.spots.map(([x, y, w, hh], i) => {
      // Mindestens so groß wie die Beschriftung im Bild, sonst so groß wie der eingesetzte Begriff: wächst nach rechts,
      // höchstens bis zur nächsten Stelle derselben Zeile bzw. zum Bildrand. Was länger ist, bricht um – abgeschnitten wird nie.
      const cy = y + hh / 2;
      const next = Math.min(1, ...ex.spots.filter(([x2, y2, , h2]) => x2 > x && Math.abs(y2 + h2 / 2 - cy) < Math.max(hh, h2, 0.06)).map(([x2]) => x2 - 0.01));
      const el = h("div.spot", { style: { left: `${x * 100}%`, top: `${cy * 100}%`, "--w": `${w * 100}%`, "--h": `${hh * 100}%`, "--max": `${(next - x) * 100}%`, "--hh": String(hh) }, role: "group", "aria-label": `Stelle ${i + 1}` }, h("span.spot-num", String(i + 1)));
      stage.append(el);
      return { el, list: el, name: `Stelle ${i + 1}`, cap: 1 };
    });
    // Start-Zoom: eingesetzte Begriffe sollen sich nicht überdecken. Schrift = 55 % der Beschriftungshöhe, mindestens 11,2 px;
    // bei Mindestschrift ist ein Begriff so breit wie seine Beschriftung, hochgerechnet auf 11,2 px. Nachbarn brauchen
    // Platz nebeneinander oder untereinander (28 px Höhe + Luft), was eben weniger Zoom kostet. Höchstens 3×, damit der Überblick bleibt.
    // ponytail: Schätzung aus den Beschriftungsfeldern; wer mehr Platz braucht, zoomt selbst weiter.
    const start = (base) => {
      const ar = img.naturalHeight / img.naturalWidth;
      const chip = (w, hh) => Math.max(48, (w * 11.2) / (0.55 * hh * ar));
      let need = 0;
      ex.spots.forEach(([x1, y1, w1, h1], i) =>
        ex.spots.slice(i + 1).forEach(([x2, y2, w2, h2]) => {
          const dx = Math.abs(x1 + w1 / 2 - (x2 + w2 / 2));
          const dy = Math.abs(y1 + h1 / 2 - (y2 + h2 / 2));
          const beside = dx ? ((chip(w1, h1) + chip(w2, h2)) / 2 + 8) / dx : Infinity;
          const below = dy ? 32 / (dy * ar) : Infinity;
          need = Math.max(need, Math.min(beside, below));
        }),
      );
      return Math.min(3, need / base) || 1;
    };
    const zoom = zoomable(stage, img, { fill: true, start });
    img.addEventListener("load", () => stage.style.setProperty("--ar", String(img.naturalHeight / img.naturalWidth)));
    img.src = ex.image;
    const bank = h("div.bank");
    const pl = placer({ options: ex.options, zones, bank, ready: (n) => n.every((x) => x === 1) });
    const words = () => zones.map((_, i) => pl.inZone(i)[0]?.textContent ?? "");
    return {
      el: h("div.stack", zoom.el, bank, pl.live),
      fit: true, // Bild füllt den freien Platz, Begriffe bleiben darunter sichtbar, auch gezoomt
      tools: zoom.bar,
      key: pl.key,
      value: () => words(),
      text: () => words().map((w, i) => `${i + 1} ${w || "–"}`).join(" · "),
      mark(ok, expected, solution) {
        pl.settle(assign(pl.chips, solution ?? []));
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
    menuBtn.hidden = !ex.id || !!battleId; // „Zuordnen“ kommt aus den Begriffen, nicht aus einer Frage; im Battle läuft die Zeit
    const view = { mc: () => choice(ex), true_false: () => choice(ex, true), fill_blank: () => blank(ex), cloze: () => blank(ex), sort: () => board(ex), order: () => chain(ex), label: () => figure(ex) }[ex.kind];
    cur.view = view ? view() : match(ex);
    const label = ex.kind === "sort" && ex.groups.length === 4 ? "Fülle die Matrix" : LABEL[ex.kind];
    body.classList.toggle("fit", !!cur.view.fit);
    body.replaceChildren(
      ...[
        h("div.q-head", h("p.q-kind", ex.retry ? `Nochmal: ${label}` : ex.refresh ? `Auffrischen: ${label}` : label), cur.view.tools),
        ["fill_blank", "cloze", "match"].includes(ex.kind) ? null : h("h1.q-prompt", ex.prompt),
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
      const text = typeof given === "string" ? given : JSON.stringify(given);
      const res = battleId ? await call("battle_answer", { battleId, idx: cur.ex.idx, given: text }) : await call("answer", { questionId: cur.ex.id, given: text, retry: !!cur.ex.retry });
      cur.view.mark(res.correct, res.expected, res.solution);
      verdict(res.correct, res.expected, res.why);
    } catch (e) {
      phase = "answer";
      setReady(true);
      fail(e);
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
      if (battleId) solved++; // im Battle zählt jede Aufgabe einmal, kein „Nochmal“
      else queue.push({ ...cur.ex, retry: true });
    }
    updateProgress();

    const nextBtn = h(`button.btn.${ok ? "right" : "wrong"}`, { type: "button", onclick: next }, "Weiter");
    const ex = cur.ex;
    const whyBtn =
      !ok && ex.id && !battleId
        ? h("button.btn.secondary", {
            type: "button",
            onclick: () =>
              openChat(ex.refresh ? null : unitId, "", {
                shown: "Warum ist das so?",
                sent: `Frage: ${ex.prompt}\nRichtige Antwort: ${expected}\nMeine Antwort: ${cur.view.text?.() ?? cur.view.value()}\nErkläre mir in 3-4 einfachen Sätzen, warum die richtige Antwort stimmt.`,
              }),
          }, icon("sparkle"), "Warum?")
        : null;
    fb.className = `feedback${ok ? "" : " wrong"}`;
    fb.replaceChildren(
      h(
        "div.inner",
        h("h2", icon(ok ? "check-circle-fill" : "x-circle-fill"), ok ? PRAISE[Math.floor(Math.random() * PRAISE.length)] : "Nicht ganz"),
        !ok && expected && CLASSIC.includes(ex.kind) ? h("p.detail", "Richtig ist: ", h("b", expected)) : null,
        why ? h("p.why", why) : null,
        h("div.actions", whyBtn, nextBtn),
      ),
    );
    requestAnimationFrame(() => {
      fb.classList.add("open");
      nextBtn.focus({ preventScroll: true });
      // Was die Leiste über der Prüfen-Zeile verdeckt, rückt per Innenabstand frei (gezoomte Grafik passt sich ein).
      root.style.setProperty("--cover", `${Math.max(0, fb.offsetHeight - foot.offsetHeight)}px`);
    });
  }

  function next() {
    if (phase !== "feedback") return;
    advance();
  }

  function advance() {
    fb.classList.remove("open");
    root.style.setProperty("--cover", "0px");
    body.classList.remove("fit");
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
      return fail(e);
    }
    const mark = h("button.btn.secondary.block", { type: "button" }, icon(q.marked ? "bookmark-fill" : "bookmark"), q.marked ? "Nicht mehr für die Klausur merken" : "Für die Klausur merken");
    const edit = h("button.btn.secondary.block", { type: "button" }, icon("pencil-simple"), "Frage bearbeiten");
    const hide = h("button.btn.secondary.block", { type: "button" }, icon("eye-slash"), "Frage ausblenden");
    const report = h("button.btn.ghost", { type: "button" }, icon("warning"), "Fehler in dieser Frage melden");
    const del = q.mine && q.shared ? h("button.btn.ghost", { type: "button", style: { color: "var(--wrong)" } }, icon("trash"), "Für alle löschen") : null;
    const s = openSheet({
      title: "Diese Frage",
      body: [
        mark,
        h("p.hint", "Gemerkte Fragen übst du gezielt unter Wiederholen → Für die Klausur."),
        h("p.muted", "Unklar formuliert oder nicht prüfungsrelevant?"),
        edit,
        hide,
        h("p.hint", "Ausgeblendet kommt sie bei dir in keiner Lektion und Wiederholung mehr vor."),
        report,
        del,
      ],
    });
    mark.addEventListener("click", async () => {
      s.close();
      try {
        await call("mark_question", { questionId: ex.id, marked: !q.marked });
      } catch (e) {
        return fail(e);
      }
      toast(q.marked ? "Nicht mehr gemerkt." : "Für die Klausur gemerkt.");
    });
    report.addEventListener("click", () => {
      s.close();
      openReport({ kind: "bug", question: q });
    });
    edit.addEventListener("click", () => {
      s.close();
      editQuestion(q, (saved) => {
        // Strukturaufgaben baut der Kern für die Lektion um: neue Fassung kommt in der nächsten Lektion.
        if (!CLASSIC.includes(ex.kind)) return drop(ex);
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
        return fail(e);
      }
      drop(ex);
      toast("Ausgeblendet.", { action: "Rückgängig", onAction: () => call("hide_question", { questionId: ex.id, hidden: false }).catch(fail) });
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
        return fail(e);
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
    if (battleId) return battleFinish();
    phase = "done";
    menuBtn.hidden = true;
    let res;
    try {
      res = await call("finish_lesson", { unitId, correct: firstRight, total });
    } catch (e) {
      fail(e);
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
    if (battleId) return battleLoad();
    phase = "loading";
    foot.hidden = true;
    body.replaceChildren(h("div.stack", h("div.skeleton", { style: { width: "40%" } }), h("div.skeleton", { style: { height: "36px" } }), h("div.skeleton", { style: { height: "62px" } }), h("div.skeleton", { style: { height: "62px" } })));
    let res;
    try {
      res = await call("start_lesson", { unitId, wrongCourse, examCourse });
    } catch (e) {
      return message("warning", "Die Lektion konnte nicht starten", String(e));
    }
    if (res.failed) {
      return message("warning", "Fragen konnten nicht erstellt werden", "Die KI hat für diesen Abschnitt keine brauchbaren Fragen geliefert.", [
        h("button.btn", { type: "button", onclick: () => call("retry_questions", { unitId }).then(wait) }, icon("arrow-clockwise"), "Neu erstellen"),
      ]);
    }
    if (!res.ready) return wait();
    if (!res.exercises.length) {
      return wrongCourse
        ? message("seal-check-fill", "Keine falschen Fragen mehr", "Was du hier richtig hattest, kommt in ein paar Tagen in der Wiederholung zurück.")
        : examCourse
          ? message("target", "Noch keine Klausurfragen", "Merke dir Fragen über ⋯ für die Klausur oder trag Prüfungsschwerpunkte im Kurs ein.")
          : message("seal-check-fill", "Gerade ist nichts fällig", "Fragen kommen zur Wiederholung zurück, kurz bevor du sie vergessen würdest.");
    }
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

  // ---------- Battle ----------
  // shortcut: Stand per Abfrage alle 1,5 s statt Server-Ereignis; SSE-Ereignis „battle-updated“, falls viele gleichzeitig spielen.
  async function battleLoad() {
    phase = "loading";
    foot.hidden = true;
    let b;
    try {
      b = await call("battle", { battleId, exercises: true });
    } catch (e) {
      return message("warning", "Battle nicht gefunden", String(e));
    }
    onBattle(b);
    poll ??= setInterval(battleTick, 1500);
    if (b.state === "waiting") return message("users", "Warte auf Gegner", "Sobald jemand deine Herausforderung annimmt, geht es los. „Zurück“ zieht sie zurück.");
    if (b.state === "expired") {
      clearInterval(poll);
      return message("hourglass-medium", "Herausforderung abgelaufen", "In 30 Minuten hat niemand angenommen. Fordere einfach neu heraus.");
    }
    if (b.state === "countdown") return countdown(b.starts_in);
    if (b.state === "done") return battleResult(b);
    queue = b.exercises.slice(b.me.answered).map((ex, i) => ({ ...ex, idx: b.me.answered + i }));
    total = b.rounds;
    solved = b.me.answered;
    updateProgress();
    if (!queue.length) return battleFinish();
    cur = { ex: queue.shift() };
    show();
  }

  async function battleTick() {
    if (ticking) return; // langsame Antwort: keine zweite Abfrage obendrauf
    ticking = true;
    const was = battle?.state;
    const b = await call("battle", { battleId }).catch(() => null);
    ticking = false;
    if (!b || !root.isConnected) return;
    onBattle(b);
    if (b.state === "done") return resultShown || battleResult(b);
    if (was === "waiting" && b.state !== "waiting") battleLoad();
  }

  /** Gegner oben neben dem eigenen Fortschritt: Ring + Name + wie weit. */
  function onBattle(b) {
    battle = b;
    const t = b.them;
    rival.hidden = !t;
    if (!t) return;
    rival.setAttribute("aria-label", `${t.name}: ${t.answered} von ${b.rounds} Aufgaben`);
    rival.replaceChildren(h("span.ring", { style: { "--p": String(t.answered / Math.max(1, b.rounds)) } }), h("b", t.name), h("span.num", `${t.answered}/${b.rounds}`));
  }

  /** 3 · 2 · 1: Die Aufgaben schickt der Server erst danach. */
  function countdown(ms) {
    clearTimeout(countTimer);
    menuBtn.hidden = true;
    const num = h("b.count-num", { "aria-live": "assertive" });
    body.replaceChildren(h("div.done-screen", h("p.muted", `Gegen ${battle.them?.name ?? "…"}`), num, h("p.muted", "An jeder Stelle dieselbe Aufgabenart. Mehr Richtige gewinnt, bei Gleichstand die kürzere Zeit.")));
    const end = Date.now() + ms;
    const step = () => {
      const left = Math.ceil((end - Date.now()) / 1000);
      if (left <= 0) return battleLoad();
      if (num.textContent !== String(left)) {
        num.textContent = String(left);
        num.animate(reduceMotion() ? [{ opacity: 0 }, { opacity: 1 }] : [{ opacity: 0, transform: "scale(0.9)" }, { opacity: 1, transform: "none" }], { duration: 250, easing: "cubic-bezier(0.23, 1, 0.32, 1)" });
      }
      countTimer = setTimeout(step, 100);
    };
    step();
  }

  function battleFinish() {
    phase = "done";
    foot.hidden = true;
    body.replaceChildren(h("div.empty", h("div.art", icon("hourglass-medium")), h("h1", `Warte auf ${battle?.them?.name ?? "deinen Gegner"}`), h("p", "Du bist durch. Das Ergebnis kommt, sobald ihr beide fertig seid.")));
    battleTick();
  }

  function battleResult(b) {
    resultShown = true;
    phase = "done";
    clearInterval(poll);
    clearTimeout(countTimer);
    fb.classList.remove("open");
    root.style.setProperty("--cover", "0px");
    body.classList.remove("fit");
    menuBtn.hidden = true;
    combo.classList.add("hide");
    const { me, them } = b;
    const title = b.result > 0 ? "Gewonnen!" : b.result < 0 ? "Verloren" : "Unentschieden";
    const why = them.gave_up
      ? `${them.name} hat aufgegeben.`
      : me.gave_up
        ? "Du hast aufgegeben."
        : b.result && me.correct === them.correct
          ? b.result > 0 ? "Gleich viele richtig, du warst schneller." : `Gleich viele richtig, ${them.name} war schneller.`
          : null;
    const time = (s) => (s.answered === b.rounds ? `${Math.floor(s.ms / 60000)}:${String(Math.floor(s.ms / 1000) % 60).padStart(2, "0")}` : "–");
    const card = (s, name, won) => h(`div${won ? ".won" : ""}`, h("b", `${s.correct}/${b.rounds}`), h("span", name), h("span.num", time(s)));
    body.replaceChildren(
      h(
        "div.done-screen",
        icon(b.result > 0 ? "crown-fill" : b.result < 0 ? "x-circle-fill" : "users", `flame battle-${b.result > 0 ? "won" : b.result < 0 ? "lost" : "draw"} ignite`),
        h("h1", title),
        why ? h("p.muted", why) : null,
        h("div.tally.duo", card(me, "Du", b.result > 0), card(them, them.name, b.result < 0)),
        b.xp ? h("p", { style: { "font-weight": "700" } }, icon("star-fill"), ` +${b.xp} XP für die Bestenliste`) : null,
        h("p.hint", them.course ? `${them.name} hat mit „${them.course}“ gespielt.` : `${them.name} hat mit einem eigenen Kurs gespielt.`),
      ),
    );
    if (b.result > 0) {
      sound.done();
      confetti();
    }
    foot.hidden = false;
    mainBtn.textContent = "Weiter";
    setReady(true);
    mainBtn.focus({ preventScroll: true });
  }

  // ---------- Steuerung ----------
  mainBtn.addEventListener("click", () => (phase === "done" ? exit() : check()));
  closeBtn.addEventListener("click", askClose);
  menuBtn.addEventListener("click", questionMenu);

  async function askClose() {
    if (battleId && battle?.state === "running" && phase !== "done") {
      const quit = await confirmSheet({ title: "Battle aufgeben?", text: "Das zählt als verloren.", confirm: "Aufgeben", cancel: "Weiterspielen", danger: true });
      if (quit) await call("battle_leave", { battleId, giveUp: true }).then(exit, fail);
      return;
    }
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
      if (e.target instanceof Element && e.target.closest(".feedback, .lesson-top, input, textarea, .piece, .zone-head")) return; // dort reagiert das Element selbst
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
    // Wartend verlassen (Zurück, Escape, Browser-Zurück, anderer Tab) → Herausforderung zurückziehen.
    // Hat inzwischen jemand angenommen, zieht der Server nicht zurück: dann das Battle wieder öffnen statt kampflos zu verlieren.
    if (battleId && battle?.state === "waiting") {
      call("battle_leave", { battleId, giveUp: false }).then((gone) => gone || (location.hash = `#/lesson/battle-${battleId}`), () => {});
    }
    unsub?.();
    clearInterval(poll);
    clearTimeout(countTimer);
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
