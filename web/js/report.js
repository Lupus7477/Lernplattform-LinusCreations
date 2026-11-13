// Fehler melden: Fehler sammeln, vorsortieren (Programmfehler oder normaler Hinweis), Issue auf GitHub.
// Mit eigenem Server (und Token dort) legt der Server das Issue an. Sonst öffnet sich ein vorausgefülltes Issue im Browser.
// Systemdaten des Geräts gehen immer mit; die des Servers hängt der Server selbst an.
import { call } from "./api.js";
import { h, icon, openExternal, openSheet, segmented, toast } from "./ui.js";
import { state } from "./state.js";

const REPO = "Lupus7477/Lernplattform-LinusCreations";
const msg = (e) => (typeof e === "string" ? e : e instanceof Error ? `${e.name}: ${e.message}` : e?.message || String(e));
const norm = (s) => String(s).toLowerCase().replace(/ä/g, "ae").replace(/ö/g, "oe").replace(/ü/g, "ue").replace(/ß/g, "ss").replace(/\s+/g, " ").trim();

let info = null;
/** { version, repo, normal } vom Kern (lokal oder Server). */
export async function appInfo() {
  info ??= await call("version").catch(() => ({ version: "?", repo: REPO, normal: [] }));
  return info;
}

// ---------- Fehler sammeln ----------
const errors = [];
function note(cmd, e) {
  errors.push({ at: new Date().toLocaleTimeString("de-DE"), cmd, message: msg(e) });
  if (errors.length > 5) errors.shift();
}
addEventListener("lernpfad:error", (ev) => note(ev.detail.cmd, ev.detail.message));
addEventListener("error", (ev) => note("js", ev.error?.stack || ev.message));
// Abgelehnte Befehle kommen schon über lernpfad:error; hier nur echte JS-Ausnahmen
addEventListener("unhandledrejection", (ev) => ev.reason instanceof Error && note("js", ev.reason.stack || ev.reason.message));

/** Meldung, die der Code selbst für Nutzer schreibt (falsches Passwort, kein Netz …): kein „Melden“. */
const isNormal = (m) => (info?.normal || []).some((p) => norm(m).includes(p));

/** Fehler als Toast. Ist es kein bekannter Nutzerhinweis, gibt es „Melden“. */
export function fail(e) {
  const message = msg(e);
  if (e?.locked || isNormal(message)) return toast(message);
  const cmd = errors.findLast((x) => x.message === message)?.cmd || "";
  toast(message, { action: "Melden", onAction: () => openReport({ error: { message, cmd } }) });
}

// ---------- Systemdaten ----------
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
    cpu_threads: n.hardwareConcurrency,
    ram_gb: n.deviceMemory ?? null,
    bildschirm: `${screen.width}×${screen.height} @${devicePixelRatio}x`,
    fenster: `${innerWidth}×${innerHeight}`,
  };
  // App: Systemdaten dieses Geräts vom lokalen Kern (auch wenn sie mit dem Server verbunden ist)
  if (tauri) d.geraet = await call("device_info").catch((e) => ({ fehler: msg(e) }));
  return d;
}

function errorText(error) {
  if (!error) return "";
  const recent = errors.map((x) => `${x.at} · ${x.cmd || "?"} · ${x.message}`).join("\n");
  return `Befehl: ${error.cmd || "?"}\nMeldung: ${error.message}${recent ? `\n\nLetzte Fehler:\n${recent}` : ""}`;
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

// ---------- Link ohne Server ----------
const MAX_URL = 8000;
/** Vorausgefülltes Issue. Zu lang? Dann wird der vordere Teil gekürzt, die Systemdaten (<details>) bleiben immer drin. */
function issueUrl(repo, { title, body, label }) {
  const url = (b) => `https://github.com/${repo}/issues/new?${new URLSearchParams({ title, body: b, labels: label })}`;
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

/** Startfehler: ohne Kern, nur mit Browserdaten. */
export function bootReport(message) {
  const device = JSON.stringify({ variante: window.__TAURI__ ? "App" : "Web-App", browser: navigator.userAgent, ansicht: location.hash }, null, 2);
  const body = `## Fehlermeldung\n\n\`\`\`text\n${String(message).replace(/`/g, "'")}\n\`\`\`\n\n<details><summary>Gerät</summary>\n\n\`\`\`json\n${device}\n\`\`\`\n</details>\n`;
  openExternal(issueUrl(REPO, { title: "App startet nicht", body, label: "bug" }));
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
  let triage = null;
  try {
    triage = await call("triage_error", { message: error.message, cmd: error.cmd });
  } catch {
    /* Server/KI weg: im Zweifel melden lassen */
  }
  if (triage && !triage.code) {
    const anyway = h("button.btn.ghost", { type: "button" }, "Trotzdem melden");
    anyway.addEventListener("click", () => form(s, body, { kind: "bug", error, triage }));
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
  const direct = onServer && state.session.report;
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
  const data = h("pre.report-data", "Wird gesammelt …");
  const details = h("details.q-box", h("summary", h("span", "Was mitgeschickt wird"), icon("caret-down")), data);
  let device = null;
  const collect = async () => (device ??= await deviceInfo());
  details.addEventListener("toggle", async () => {
    if (!details.open) return;
    const d = await collect();
    data.textContent = [errorText(error), question && withQ.checked ? questionText(question) : "", JSON.stringify(d, null, 2), onServer ? "+ Systemdaten des Servers" : ""].filter(Boolean).join("\n\n");
  });
  const send = h("button.btn.block", { type: "button" }, icon("paper-plane-right"), direct ? "Senden" : "Weiter zu GitHub");

  body.replaceChildren(
    ...[
    error ? h("div.notice.error", icon("warning"), h("b", "Fehlermeldung"), h("span", error.message)) : segmented([["bug", "Fehler"], ["idea", "Idee"]], kind, setKind, "Art der Meldung"),
    triage?.hint ? h("p.muted", triage.hint) : null,
    h("div.field", label, text),
    question
      ? h("div.card.report-q", h("p", h("b", "Frage: "), question.prompt), h("div.row.between", h("label", { for: "report-q" }, "Frage mitschicken"), withQ))
      : null,
    h(
      "p.hint",
      "Mit gehen Systemdaten von Gerät",
      onServer ? " und Server" : "",
      " sowie Fehlermeldungen, aber kein Name und keine IP. ",
      direct ? "Daraus wird direkt ein Issue auf GitHub, die KI schreibt die Beschreibung." : "GitHub öffnet sich im Browser, zum Absenden brauchst du dort ein Konto.",
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
    send.disabled = true;
    const label0 = send.lastChild.textContent;
    send.lastChild.textContent = "Die KI schreibt die Meldung …";
    const payload = {
      kind,
      text: text.value.trim(),
      error: errorText(error),
      question: question && withQ.checked ? questionText(question) : "",
      device: await collect(),
      triage,
      message: error?.message || "",
      cmd: error?.cmd || "",
    };
    try {
      if (direct) {
        try {
          const r = await call("report_issue", payload);
          s.close();
          return toast(`Danke! Gemeldet als #${r.number}.`, { action: "Ansehen", onAction: () => openExternal(r.url) });
        } catch (e) {
          if (e?.locked) throw e;
          // GitHub lehnt ab oder ist weg: dann eben im Browser
          toast(`Direkt senden ging nicht (${msg(e)}). GitHub öffnet sich im Browser.`);
        }
      }
      const d = await call("draft_issue", payload);
      openExternal(issueUrl(info?.repo || REPO, d));
      s.close();
      toast("GitHub ist geöffnet. Dort noch auf „Create“ tippen.");
    } catch (e) {
      err.textContent = msg(e);
      err.hidden = false;
      send.disabled = false;
      send.lastChild.textContent = label0;
    }
  });
}
