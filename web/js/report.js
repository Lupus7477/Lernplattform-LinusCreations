// Fehler melden: Fehler sammeln, vorsortieren (Programmfehler oder normaler Hinweis), Issue auf GitHub.
// Mit eigenem Server (und Token dort) legt der Server das Issue an, bei Programmfehlern auch automatisch.
// Sonst entsteht ein vorausgefülltes Issue, das man über einen Link auf GitHub absendet.
// Systemdaten des Geräts gehen immer mit; die des Servers hängt der Server selbst an.
import { call, message } from "./api.js";
import { h, icon, openExternal, openSheet, segmented, toast } from "./ui.js";
import { state } from "./state.js";

const REPO = "Lupus7477/Lernplattform-LinusCreations";
const norm = (s) => String(s).toLowerCase().replace(/ä/g, "ae").replace(/ö/g, "oe").replace(/ü/g, "ue").replace(/ß/g, "ss").replace(/\s+/g, " ").trim();
const clip = (s, n) => (s.length > n ? `${s.slice(0, n)} … (gekürzt)` : s);

let info = null;
/** { version, repo, normal } vom Kern (lokal oder Server). */
export async function appInfo() {
  info ??= await call("version").catch(() => ({ version: "?", repo: REPO, normal: [] }));
  return info;
}
const repo = () => info?.repo || REPO;
/** Darf der Server Issues selbst anlegen? (eigener Server mit GitHub-Token) */
const serverPosts = () => state.session?.mode === "server" && !!state.session.report;

// ---------- Fehler sammeln ----------
const errors = [];
function note(cmd, text) {
  errors.push({ at: new Date().toLocaleTimeString("de-DE"), cmd, message: text });
  if (errors.length > 5) errors.shift();
}
addEventListener("lernpfad:error", (ev) => note(ev.detail.cmd, ev.detail.message));
addEventListener("error", (ev) => note("js", ev.error?.stack || ev.message));
// Nicht abgefangene Fehler sollen nie still verschwinden
addEventListener("unhandledrejection", (ev) => {
  if (ev.reason instanceof Error) note("js", ev.reason.stack || ev.reason.message);
  fail(ev.reason);
});

/** Hinweis an Nutzer statt Programmfehler: offline, Sperre, oder ein Text, den der Code selbst für Nutzer schreibt. */
const isNormal = (e, text) => e?.locked || navigator.onLine === false || (info?.normal || []).some((p) => norm(text).includes(p));

/**
 * Fehler anzeigen. Programmfehler meldet die App auf dem eigenen Server selbst (die KI schätzt erst ein);
 * sonst bietet der Toast „Melden“ an. Hinweise für Nutzer (falsches Passwort, offline …) bleiben ein einfacher Toast.
 */
export function fail(e) {
  const text = message(e);
  if (isNormal(e, text)) return toast(text, { error: true });
  const error = { message: text, cmd: errors.findLast((x) => x.message === text)?.cmd || "" };
  toast(text, { error: true, action: "Melden", onAction: () => openReport({ error }) });
  if (serverPosts()) autoReport(error);
}

// ---------- Automatisch melden ----------
const autoSent = new Set();
/** Programmfehler im Hintergrund melden. Ohne sichere Einschätzung (KI belegt) bleibt es beim „Melden“ im Toast. */
async function autoReport(error) {
  const key = `${error.cmd}\n${error.message}`;
  if (autoSent.has(key)) return;
  autoSent.add(key);
  try {
    const triage = await call("triage_error", error);
    if (!triage.code) return triage.hint && toast(triage.hint);
    if (triage.by === "ohne_ki") return;
    const r = await call("report_issue", await payload({ kind: "bug", error, triage, auto: true }));
    toast(r.known ? `Dieser Fehler ist schon gemeldet (#${r.number}).` : `Fehler automatisch gemeldet (#${r.number}).`, { action: "Ansehen", onAction: () => openExternal(r.url) });
  } catch (e) {
    autoSent.delete(key);
    console.warn("Automatisch melden ging nicht:", message(e));
  }
}

// ---------- Inhalt einer Meldung ----------
async function deviceInfo() {
  const n = navigator;
  const tauri = !!window.__TAURI__;
  const mobile = /Android|iPhone|iPad|iPod/i.test(n.userAgent);
  const s = state.session;
  const d = {
    variante: tauri ? (mobile ? "Handy-App" : "PC-App") : matchMedia("(display-mode: standalone)").matches ? "Web-App (installiert)" : "Web-App",
    verbindung: s?.mode === "server" ? (s.remote ? "App mit eigenem Server" : "Web vom eigenen Server") : "lokal",
    ansicht: location.hash || "#/",
    browser: n.userAgent,
    plattform: n.userAgentData?.platform || n.platform,
    sprache: n.language,
    online: n.onLine,
    cpu_threads: n.hardwareConcurrency,
    ram_gb: n.deviceMemory ?? null,
    bildschirm: `${screen.width}×${screen.height} @${devicePixelRatio}x`,
    fenster: `${innerWidth}×${innerHeight}`,
  };
  // App: Systemdaten dieses Geräts vom lokalen Kern (auch wenn sie mit dem Server verbunden ist)
  if (tauri) d.geraet = await call("device_info").catch((e) => ({ fehler: message(e) }));
  return d;
}

function errorText(error) {
  if (!error) return "";
  const recent = errors.map((x) => `${x.at} · ${x.cmd || "?"} · ${clip(x.message, 600)}`).join("\n");
  return `Befehl: ${error.cmd || "?"}\nMeldung: ${clip(error.message, 2000)}${recent ? `\n\nLetzte Fehler:\n${recent}` : ""}`;
}

function questionText(q) {
  if (!q) return "";
  const lines = [`Frage-ID: ${q.id} · Einheit: ${q.unit_id}`, `Typ: ${q.kind}`, `Frage: ${q.prompt}`];
  if (q.options?.length) lines.push(`Optionen: ${q.options.join(" | ")}`);
  if (q.groups?.length) lines.push(`Felder: ${q.groups.map((g) => `${g.name}: ${g.items.join(", ")}`).join(" · ")}`);
  if (q.spots?.length) lines.push(`Stellen: ${q.spots.map((s) => s.label).join(", ")}`);
  lines.push(`Antwort: ${q.answer}`);
  if (q.why) lines.push(`Begründung: ${q.why}`);
  return lines.join("\n");
}

/** Alles, was an Kern oder Server geht (`report::Input`). */
async function payload({ kind, text = "", error = null, question = null, triage = null, auto = false }) {
  return {
    kind,
    text,
    error: errorText(error),
    question: questionText(question),
    device: await deviceInfo(),
    triage,
    message: error?.message || "",
    cmd: error?.cmd || "",
    auto,
  };
}

// ---------- Issue über einen Link (ohne Server, Server ohne Token oder wenn GitHub direkt nicht ging) ----------
const fence = (lang, s) => `\`\`\`${lang}\n${String(s).replace(/`/g, "'")}\n\`\`\`\n`;

/** Issue-Text ohne Kern (Server weg, App startet nicht): Originaltext, Fehler, Frage und Gerät. */
function localDraft(p) {
  const parts = ["_Ohne KI erstellt: Die App konnte den Kern nicht erreichen._\n"];
  if (p.text) parts.push(`## Originaltext\n\n${fence("text", p.text)}`);
  if (p.error) parts.push(`## Fehlermeldung\n\n${fence("text", p.error)}`);
  if (p.question) parts.push(`## Frage\n\n${fence("text", p.question)}`);
  parts.push(`<details><summary>Gerät</summary>\n\n${fence("json", JSON.stringify(p.device, null, 2))}</details>\n`);
  const first = (p.text || p.message || "Fehler").split("\n").find((l) => l.trim()) || "Fehler";
  return { title: clip(first.trim(), 70), body: parts.join("\n"), label: p.kind === "idea" ? "enhancement" : "bug" };
}

const MAX_URL = 8000;
/** Vorausgefülltes Issue. Zu lang? Dann wird der vordere Teil gekürzt, die Systemdaten (<details>) bleiben immer drin. */
function issueUrl({ title, body, label }) {
  const url = (b) => `https://github.com/${repo()}/issues/new?${new URLSearchParams({ title, body: b, labels: label })}`;
  if (url(body).length <= MAX_URL) return url(body);
  const cut = body.indexOf("\n<details>");
  const [head, tail] = cut < 0 ? [body, ""] : [body.slice(0, cut), body.slice(cut)];
  let keep = head.length;
  let short;
  do {
    keep = Math.floor(keep * 0.8);
    let part = head.slice(0, keep);
    if ((part.match(/```/g) || []).length % 2) part += "\n```"; // offenen Code-Block schließen
    short = `${part}\n\n_… gekürzt_\n${tail}`;
  } while (url(short).length > MAX_URL && keep > 200);
  return url(short);
}

/** Link zum Absenden auf GitHub. Ein echter Link (Klick des Nutzers) wird von keinem Browser blockiert, in der App öffnet ihn das Opener-Plugin. */
function githubStep(body, s, draft, note) {
  const link = h("a.btn.block", { href: issueUrl(draft), target: "_blank", rel: "noopener", onclick: () => s.close() }, icon("link-simple"), "Auf GitHub absenden");
  body.replaceChildren(
    ...[
      note ? h("p.form-error", { role: "alert" }, note) : null,
      h("div.notice", icon("check-circle-fill"), h("b", "Die Meldung ist fertig"), h("span", draft.title)),
      h("p.muted", "GitHub öffnet sich mit allem vorausgefüllt. Dort noch auf „Create“ tippen. Zum Absenden brauchst du ein GitHub-Konto."),
      link,
    ].filter(Boolean),
  );
  link.focus();
}

/** Startfehler: ohne Kern, nur mit Browserdaten. */
export async function bootReport(text) {
  const p = { kind: "bug", text: "", error: `Meldung: ${clip(text, 4000)}`, question: "", message: "App startet nicht", device: { variante: window.__TAURI__ ? "App" : "Web-App", browser: navigator.userAgent, ansicht: location.hash } };
  const body = h("div.stack");
  githubStep(body, openSheet({ title: "Fehler melden", body }), { ...localDraft(p), title: "App startet nicht" });
}

// ---------- Melden ----------
/**
 * Meldung an die Entwickler. `error` = { message, cmd } aus einem Fehler-Toast: erst vorsortieren.
 * `question` = Frage aus dem Fragen-Menü (wird angezeigt und mitgeschickt, abschaltbar).
 */
export async function openReport({ kind = "bug", question = null, error = null } = {}) {
  await appInfo();
  const body = h("div.stack");
  const s = openSheet({ title: error ? "Fehler melden" : kind === "idea" ? "Idee vorschlagen" : "Fehler melden", body });
  if (!error) return form(s, body, { kind, question });

  body.replaceChildren(h("p.muted", { role: "status" }, "Wird geprüft …"), h("div.skeleton", { style: { height: "64px" } }));
  const triage = await call("triage_error", error).catch(() => null); // Server oder KI weg: im Zweifel melden lassen
  if (triage && !triage.code) {
    const anyway = h("button.btn.ghost", { type: "button", onclick: () => form(s, body, { kind: "bug", error, triage }) }, "Trotzdem melden");
    body.replaceChildren(
      ...[
        h("div.notice", icon("check-circle-fill"), h("b", "Das liegt nicht am Programm"), h("span", error.message)),
        triage.hint ? h("p", triage.hint) : null,
        h("button.btn.block", { type: "button", onclick: () => s.close() }, "Verstanden"),
        anyway,
      ].filter(Boolean),
    );
    return;
  }
  form(s, body, { kind: "bug", error, triage });
}

function form(s, body, { kind, question = null, error = null, triage = null }) {
  const onServer = state.session?.mode === "server";
  const direct = serverPosts();
  const text = h("textarea.input", { id: "report-text", rows: 5, maxlength: 4000, "aria-describedby": "report-err" });
  const label = h("label", { for: "report-text" });
  const err = h("p.form-error", { id: "report-err", role: "alert", hidden: true });
  const setKind = (k) => {
    kind = k;
    label.textContent = error ? "Was hast du gerade gemacht? (optional)" : k === "idea" ? "Was wünschst du dir?" : "Was ist passiert?";
    text.placeholder = k === "idea" ? "z. B. Ich würde gern … , weil …" : "Was hast du gemacht, was ist passiert, was hättest du erwartet?";
  };
  setKind(kind);
  const withQ = h("input.switch", { type: "checkbox", id: "report-q", checked: true });
  const collect = () => payload({ kind, text: text.value.trim(), error, question: question && withQ.checked ? question : null, triage });
  const data = h("pre.report-data", "Wird gesammelt …");
  const details = h("details.q-box", h("summary", h("span", "Was mitgeschickt wird"), icon("caret-down")), data);
  details.addEventListener("toggle", async () => {
    if (!details.open) return;
    const p = await collect();
    data.textContent = [p.error, p.question, JSON.stringify(p.device, null, 2), onServer ? "+ Systemdaten des Servers" : ""].filter(Boolean).join("\n\n");
  });
  const send = h("button.btn.block", { type: "button" }, icon("paper-plane-right"), h("span", direct ? "Senden" : "Weiter"));

  body.replaceChildren(
    ...[
      error ? h("div.notice.error", icon("warning"), h("b", "Fehlermeldung"), h("span", error.message)) : segmented([["bug", "Fehler"], ["idea", "Idee"]], kind, setKind, "Art der Meldung"),
      triage?.hint ? h("p.muted", triage.hint) : null,
      h("div.field", label, text),
      question ? h("div.card.report-q", h("p", h("b", "Frage: "), question.prompt), h("div.row.between", h("label", { for: "report-q" }, "Frage mitschicken"), withQ)) : null,
      h(
        "p.hint",
        `Mit gehen Systemdaten von Gerät${onServer ? " und Server" : ""} sowie Fehlermeldungen, aber kein Name und keine IP. `,
        direct ? "Daraus wird direkt ein Issue auf GitHub, die KI schreibt die Beschreibung." : "Danach sendest du es über GitHub ab (dafür brauchst du dort ein Konto).",
      ),
      details,
      err,
      send,
    ].filter(Boolean),
  );
  text.focus();

  send.addEventListener("click", async () => {
    err.hidden = true;
    if (!error && text.value.trim().length < 10) {
      err.textContent = "Beschreib kurz in ein paar Worten, worum es geht.";
      err.hidden = false;
      return text.focus();
    }
    const idle = send.lastChild.textContent;
    send.disabled = true;
    send.lastChild.textContent = "Die KI schreibt die Meldung …";
    const p = await collect();
    let note = "";
    if (direct) {
      try {
        const r = await call("report_issue", p);
        s.close();
        return toast(`Danke! Gemeldet als #${r.number}.`, { action: "Ansehen", onAction: () => openExternal(r.url) });
      } catch (e) {
        if (e?.locked) {
          err.textContent = message(e);
          err.hidden = false;
          send.disabled = false;
          send.lastChild.textContent = idle;
          return;
        }
        note = `Direkt senden ging nicht: ${message(e)}`;
      }
    }
    // Ohne Server-Token (oder wenn GitHub direkt nicht ging): Entwurf vom Kern, sonst ohne KI hier gebaut
    const draft = await call("draft_issue", p).catch(() => localDraft(p));
    githubStep(body, s, draft, note);
  });
}
