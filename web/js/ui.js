// Kleine UI-Helfer ohne Framework. Texte immer als Textknoten → kein HTML aus KI-Antworten im DOM.

export function h(tag, props, ...kids) {
  if (props == null || props instanceof Node || typeof props !== "object" || Array.isArray(props)) {
    kids.unshift(props);
    props = {};
  }
  const [name, ...classes] = tag.split(".");
  const el = document.createElement(name || "div");
  if (classes.length) el.className = classes.join(" ");
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.classList.add(...String(v).split(" ").filter(Boolean));
    else if (k === "style") for (const [sk, sv] of Object.entries(v)) el.style.setProperty(sk, sv);
    else if (k === "value") el.value = v;
    else if (k in el && typeof v !== "string") el[k] = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  append(el, kids);
  return el;
}

export function append(el, kids) {
  for (const k of [kids].flat(Infinity)) {
    if (k == null || k === false) continue;
    el.append(k instanceof Node ? k : String(k));
  }
  return el;
}

export function icon(name, cls = "") {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", `icon ${cls}`.trim());
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

export const reduceMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

// ---------- Gerätespeicher für Komfort-Einstellungen ----------
export const pref = {
  get(key, fallback) {
    try {
      return localStorage.getItem(key) ?? fallback;
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* privat/blockiert: Einstellung gilt nur für diese Sitzung */
    }
  },
};

export function applyTheme(mode = pref.get("theme", "system")) {
  if (mode === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = mode;
}

/** Link im System-Browser öffnen (App: Opener-Plugin, Web: neuer Tab). */
export function openExternal(url) {
  const opener = window.__TAURI__?.opener;
  if (opener) return opener.openUrl(url).catch(() => window.open(url, "_blank", "noopener"));
  window.open(url, "_blank", "noopener");
}

// ---------- Rückmeldungen ----------
const TOAST_MS = { info: 4000, action: 7000, error: 12000 };
let toasts;

/** Zeitgeber, den man anhalten kann (Toast unter Maus oder Fokus bleibt stehen). */
function pausable(fn, ms) {
  let left = ms;
  let started = 0;
  let id = 0;
  const resume = () => {
    started = Date.now();
    id = setTimeout(fn, left);
  };
  const pause = () => {
    clearTimeout(id);
    left = Math.max(1500, left - (Date.now() - started));
  };
  resume();
  return { pause, resume, stop: () => clearTimeout(id) };
}

/**
 * Kurze Meldung unten. Mehrere stapeln sich statt sich zu verdecken.
 * `action` (z. B. „Rückgängig“) und `error` (Fehler, mit ×) bleiben länger stehen; unter Maus oder Fokus bleibt jede stehen.
 */
export function toast(message, { action, onAction, error = false } = {}) {
  toasts ??= document.body.appendChild(h("div.toasts"));
  const el = h(`div.toast${error ? ".is-error" : ""}`, { role: error ? "alert" : "status" }, h("span.toast-text", message));
  let gone = false;
  const hide = () => {
    if (gone) return;
    gone = true;
    timer.stop();
    el.style.opacity = "0";
    setTimeout(() => el.remove(), 220);
  };
  if (action) el.append(h("button.toast-action", { type: "button", onclick: () => (hide(), onAction()) }, action));
  if (error) el.append(h("button.toast-close", { type: "button", "aria-label": "Meldung schließen", onclick: hide }, icon("x")));
  toasts.append(el);
  while (toasts.childElementCount > 3) toasts.firstElementChild.remove();
  const timer = pausable(hide, error ? TOAST_MS.error : action ? TOAST_MS.action : TOAST_MS.info);
  el.addEventListener("pointerenter", timer.pause);
  el.addEventListener("pointerleave", timer.resume);
  el.addEventListener("focusin", timer.pause);
  el.addEventListener("focusout", timer.resume);
  return { hide };
}

/** Umschalter aus Knöpfen (aria-pressed). `options` = [[wert, text], …] */
export function segmented(options, value, onPick, label) {
  const btns = options.map(([v, text]) => h("button", { type: "button", "aria-pressed": String(v === value) }, text));
  btns.forEach((b, i) =>
    b.addEventListener("click", () => {
      btns.forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
      onPick(options[i][0]);
    }),
  );
  return h("div.segmented", { role: "group", "aria-label": label }, btns);
}

let audio;
function tone(freqs, { type = "sine", dur = 0.12, gap = 0.085, vol = 0.16 } = {}) {
  if (pref.get("sound", "on") !== "on") return;
  try {
    audio ??= new AudioContext();
    const t0 = audio.currentTime + 0.01;
    freqs.forEach((f, i) => {
      const osc = audio.createOscillator();
      const gain = audio.createGain();
      osc.type = type;
      osc.frequency.value = f;
      const t = t0 + i * gap;
      gain.gain.setValueAtTime(0.0001, t);
      gain.gain.exponentialRampToValueAtTime(vol, t + 0.012);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
      osc.connect(gain).connect(audio.destination);
      osc.start(t);
      osc.stop(t + dur + 0.02);
    });
  } catch {
    /* kein Audio verfügbar */
  }
}
export const sound = {
  right: () => tone([659, 988]),
  wrong: () => tone([233, 185], { type: "triangle", dur: 0.18, gap: 0.12, vol: 0.14 }),
  done: () => tone([523, 659, 784, 1047], { dur: 0.16, gap: 0.1 }),
};

/** Kurzes Wackeln bei falscher Antwort (einmalig, daher Keyframes ok). */
export function shake(el) {
  if (reduceMotion()) return;
  el.animate(
    [{ transform: "translateX(0)" }, { transform: "translateX(-7px)" }, { transform: "translateX(6px)" }, { transform: "translateX(-4px)" }, { transform: "translateX(0)" }],
    { duration: 300, easing: "cubic-bezier(0.23, 1, 0.32, 1)" },
  );
}

/**
 * Elemente gleiten vom alten an den neuen Platz (FLIP): messen, DOM ändern, vom alten Ort zurück animieren.
 * Laufende Bewegungen werden unterbrochen, damit nichts springt, wenn man schnell weitertippt.
 */
export function flip(els, mutate, ms = 240) {
  const first = new Map(els.map((e) => [e, e.getBoundingClientRect()]));
  els.forEach((e) => e.getAnimations().forEach((a) => a.cancel()));
  mutate();
  if (reduceMotion()) return;
  for (const [e, r] of first) {
    const n = e.getBoundingClientRect();
    const dx = r.left - n.left;
    const dy = r.top - n.top;
    if (Math.abs(dx) + Math.abs(dy) < 1) continue;
    e.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], { duration: ms, easing: "cubic-bezier(0.23, 1, 0.32, 1)" });
  }
}

/**
 * Bild zum Hineinzoomen: zwei Finger, Strg/⌘ + Mausrad (auch Trackpad-Pinch) oder die Knöpfe −, 100 %, +.
 * Zoom ändert die Breite der Bühne statt `transform`: Stellen rücken auseinander, Begriffe darin bleiben gleich groß.
 * Zoom 1 = eingepasst in die Breite und in `maxH` der Fensterhöhe, mit `fill` in die Höhe, die das Layout dem Rahmen gibt.
 * `start(grundbreite)` → Zoom nach dem Laden. `bar` (−, 100 %, +) darf der Aufrufer woanders einhängen.
 */
export function zoomable(stage, img, { maxH = 0.6, fill = false, start } = {}) {
  const frame = h("div.figure-wrap", stage);
  const minus = h("button.icon-btn", { type: "button", "aria-label": "Verkleinern" }, icon("magnifying-glass-minus"));
  const pct = h("button.zoom-pct.num", { type: "button", "aria-label": "Bild einpassen" }, "100 %");
  const plus = h("button.icon-btn", { type: "button", "aria-label": "Vergrößern" }, icon("magnifying-glass-plus"));
  const bar = h("div.zoom-bar", minus, pct, plus);
  const el = h("div.figure-box", { style: { "--fig-max": `${maxH * 100}dvh` } }, frame, bar);
  let z = 1;
  let base = 0;
  let size = "";
  let anim = 0;

  const fit = () => {
    size = `${frame.clientWidth}x${fill ? frame.clientHeight : innerHeight}`;
    const ratio = img.naturalWidth / img.naturalHeight || 1;
    const room = fill ? frame.clientHeight - 28 : innerHeight * maxH; // 28 = Innenabstand des Rahmens
    base = Math.max(120, Math.min(frame.clientWidth - 28, room * ratio));
  };
  /** Zoom setzen; der Punkt (x, y) im Fenster bleibt dabei unter Finger bzw. Mauszeiger stehen. */
  function set(next, x, y) {
    if (!base) fit();
    const f = frame.getBoundingClientRect();
    const px = x ?? f.left + f.width / 2;
    const py = y ?? f.top + f.height / 2;
    const old = stage.getBoundingClientRect();
    const fx = (px - old.left) / (old.width || 1);
    const fy = (py - old.top) / (old.height || 1);
    z = Math.min(4, Math.max(1, next || 1));
    frame.classList.toggle("zoomed", z > 1.01);
    stage.style.width = `${Math.round(base * z)}px`;
    const now = stage.getBoundingClientRect();
    frame.scrollLeft += now.left + fx * now.width - px;
    frame.scrollTop += now.top + fy * now.height - py;
    pct.textContent = `${Math.round(z * 100)} %`;
    minus.disabled = z <= 1.01;
    plus.disabled = z >= 3.99;
  }
  /** Knöpfe: kurz gleiten statt springen, jede neue Geste unterbricht. */
  function glide(target) {
    cancelAnimationFrame(anim);
    if (reduceMotion()) return set(target);
    const from = z;
    const t0 = performance.now();
    const step = (now) => {
      const t = Math.min(1, (now - t0) / 180);
      set(from + (target - from) * (1 - (1 - t) ** 3));
      if (t < 1) anim = requestAnimationFrame(step);
    };
    anim = requestAnimationFrame(step);
  }
  minus.addEventListener("click", () => glide(z / 1.5));
  plus.addEventListener("click", () => glide(z * 1.5));
  pct.addEventListener("click", () => glide(1));

  frame.addEventListener(
    "wheel",
    (e) => {
      if (!e.ctrlKey && !e.metaKey) return; // normales Rad scrollt weiter
      e.preventDefault();
      cancelAnimationFrame(anim);
      set(z * Math.exp(-Math.max(-50, Math.min(50, e.deltaY)) * 0.01), e.clientX, e.clientY);
    },
    { passive: false },
  );
  // Zwei Finger zoomen, einer scrollt wie gewohnt.
  let pinch = null;
  const spread = ([a, b]) => Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
  frame.addEventListener("touchstart", (e) => e.touches.length === 2 && (pinch = { d: spread(e.touches), z }), { passive: true });
  frame.addEventListener(
    "touchmove",
    (e) => {
      if (!pinch || e.touches.length !== 2) return;
      e.preventDefault();
      cancelAnimationFrame(anim);
      const [a, b] = e.touches;
      set((pinch.z * spread(e.touches)) / pinch.d, (a.clientX + b.clientX) / 2, (a.clientY + b.clientY) / 2);
    },
    { passive: false },
  );
  frame.addEventListener("touchend", (e) => e.touches.length < 2 && (pinch = null));

  const load = () => {
    base = 0;
    fit();
    set(start?.(base) ?? 1);
  };
  img.addEventListener("load", load);
  if (img.complete && img.naturalWidth) requestAnimationFrame(load);
  // Drehen, Fenstergröße, Feedback-Leiste: neu einpassen, der Zoomfaktor bleibt.
  new ResizeObserver(() => {
    const now = `${frame.clientWidth}x${fill ? frame.clientHeight : innerHeight}`;
    if (frame.clientWidth && now !== size && img.naturalWidth) (fit(), set(z));
  }).observe(frame);
  return { el, frame, bar, set };
}

export function countUp(el, to, ms = 700) {
  if (reduceMotion() || to <= 0) {
    el.textContent = String(to);
    return;
  }
  const start = performance.now();
  const step = (now) => {
    const t = Math.min(1, (now - start) / ms);
    el.textContent = String(Math.round(to * (1 - Math.pow(1 - t, 3))));
    if (t < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

/** Konfetti nur am Lektionsende (seltener Moment = Platz für Freude). */
export function confetti() {
  if (reduceMotion()) return;
  const canvas = h("canvas.confetti", { "aria-hidden": "true" });
  document.body.append(canvas);
  const dpr = devicePixelRatio || 1;
  canvas.width = innerWidth * dpr;
  canvas.height = innerHeight * dpr;
  const ctx = canvas.getContext("2d");
  ctx.scale(dpr, dpr);
  const css = getComputedStyle(document.documentElement);
  const colors = ["--royal", "--marker", "--ember", "--right"].map((v) => css.getPropertyValue(v).trim());
  const parts = Array.from({ length: 80 }, (_, i) => ({
    x: innerWidth / 2 + (Math.random() - 0.5) * 80,
    y: innerHeight * 0.38,
    vx: (Math.random() - 0.5) * 13,
    vy: -Math.random() * 13 - 5,
    r: Math.random() * Math.PI,
    vr: (Math.random() - 0.5) * 0.35,
    w: 7 + Math.random() * 7,
    c: colors[i % colors.length],
  }));
  const t0 = performance.now();
  const frame = (now) => {
    const life = (now - t0) / 1600;
    ctx.clearRect(0, 0, innerWidth, innerHeight);
    for (const p of parts) {
      p.vy += 0.38;
      p.vx *= 0.985;
      p.x += p.vx;
      p.y += p.vy;
      p.r += p.vr;
      ctx.save();
      ctx.globalAlpha = Math.max(0, 1 - life);
      ctx.translate(p.x, p.y);
      ctx.rotate(p.r);
      ctx.fillStyle = p.c;
      ctx.fillRect(-p.w / 2, -p.w / 4, p.w, p.w / 2);
      ctx.restore();
    }
    if (life < 1) requestAnimationFrame(frame);
    else canvas.remove();
  };
  requestAnimationFrame(frame);
}

// ---------- Markdown (KI-Text) ----------
const esc = (s) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
let markedReady = false;
export function markdown(text) {
  const el = h("div.markdown");
  if (window.marked) {
    if (!markedReady) {
      // Rohes HTML aus der KI-Antwort wird als Text angezeigt, nie ausgeführt.
      window.marked.use({ gfm: true, breaks: false, renderer: { html: (raw) => esc(typeof raw === "string" ? raw : raw.text) } });
      markedReady = true;
    }
    el.innerHTML = window.marked.parse(text);
    el.querySelectorAll("a").forEach((a) => a.replaceWith(a.textContent));
  } else {
    el.textContent = text;
  }
  return el;
}

// ---------- Bottom-Sheet mit Wisch-zum-Schließen ----------
export function openSheet({ title, body, foot, label, onClose }) {
  const before = document.activeElement;
  const scrim = h("div.scrim");
  const closeBtn = h("button.icon-btn", { type: "button", "aria-label": "Schließen" }, icon("x"));
  const head = h("div.sheet-head", h("h2", title), closeBtn);
  const sheet = h("section.sheet", { role: "dialog", "aria-modal": "true", "aria-label": label || title }, head, h("div.sheet-body", body), foot ? h("div.sheet-foot", foot) : null);
  document.body.append(scrim, sheet);
  requestAnimationFrame(() => {
    scrim.classList.add("open");
    sheet.classList.add("open");
    (sheet.querySelector("[autofocus]") || sheet.querySelector(".sheet-foot button, .sheet-body button, input, textarea") || closeBtn).focus({ preventScroll: true });
  });

  let closed = false;
  function close() {
    if (closed) return;
    closed = true;
    sheet.style.transform = "";
    sheet.classList.remove("open");
    scrim.classList.remove("open");
    document.removeEventListener("keydown", onKey, true);
    setTimeout(() => {
      sheet.remove();
      scrim.remove();
    }, 220);
    before?.focus?.({ preventScroll: true });
    onClose?.();
  }
  function onKey(e) {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  }
  document.addEventListener("keydown", onKey, true);
  scrim.addEventListener("click", close);
  closeBtn.addEventListener("click", close);

  // Ziehen am Kopf: folgt dem Finger 1:1, schneller Wisch oder weit genug → schließen.
  let start = null;
  head.addEventListener("pointerdown", (e) => {
    if (e.target.closest("button")) return;
    start = { y: e.clientY, t: performance.now() };
    head.setPointerCapture(e.pointerId);
    sheet.style.transition = "none";
  });
  head.addEventListener("pointermove", (e) => {
    if (!start) return;
    const dy = e.clientY - start.y;
    const y = dy > 0 ? dy : -Math.sqrt(-dy) * 2; // nach oben: Gummiband-Widerstand
    sheet.style.transform = `translateY(${y}px)`;
  });
  const end = (e) => {
    if (!start) return;
    const dy = e.clientY - start.y;
    const v = dy / (performance.now() - start.t);
    start = null;
    sheet.style.transition = "";
    if (dy > 140 || v > 0.5) close();
    else sheet.style.transform = "";
  };
  head.addEventListener("pointerup", end);
  head.addEventListener("pointercancel", end);

  return { close, sheet };
}

/** Rückfrage vor endgültigen Aktionen. Gibt true zurück, wenn bestätigt. */
export function confirmSheet({ title, text, confirm, cancel = "Abbrechen", danger = false }) {
  return new Promise((resolve) => {
    let result = false;
    const yes = h(`button.btn.block${danger ? ".wrong" : ""}`, { type: "button" }, confirm);
    const no = h("button.btn.secondary.block", { type: "button", autofocus: true }, cancel);
    const s = openSheet({ title, body: h("p.muted", text), foot: [no, yes], onClose: () => resolve(result) });
    yes.addEventListener("click", () => {
      result = true;
      s.close();
    });
    no.addEventListener("click", () => s.close());
  });
}

export function fmtDate(iso) {
  const d = new Date(`${iso}T12:00:00`);
  return d.toLocaleDateString("de-DE", { day: "numeric", month: "long", year: "numeric" });
}

export function daysUntil(iso) {
  const today = new Date();
  today.setHours(12, 0, 0, 0);
  return Math.round((new Date(`${iso}T12:00:00`) - today) / 86_400_000);
}

export const gb = (bytes) => (bytes / 1e9).toLocaleString("de-DE", { maximumFractionDigits: 1, minimumFractionDigits: 1 });

/** „VL01_Markt_und_Preis.pdf“ → „VL01 Markt und Preis“ */
export const docName = (file) => file.replace(/\.pdf$/i, "").replace(/[_]+/g, " ").trim();
