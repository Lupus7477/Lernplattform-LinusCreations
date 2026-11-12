// Profil: Streak, Kalender, Woche, Einstellungen, Konto/Server, KI.
import { call } from "./api.js";
import { PASSWORD_RULES } from "./auth.js";
import { applyTheme, confirmSheet, gb, h, icon, openSheet, pref, segmented, toast } from "./ui.js";
import { aiNotice, emit, loadStats, state, subscribe } from "./state.js";

const DOW = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const iso = (d) => d.toLocaleDateString("sv-SE"); // JJJJ-MM-TT in lokaler Zeit

export async function render() {
  const page = h("div.page");
  const [stats, tiers] = await Promise.all([loadStats(), call("ai_tiers")]);
  // Konto-Karten einmal bauen: das Neuzeichnen bei KI-Statusmeldungen soll nicht jedes Mal den Server fragen.
  const user = state.session?.user;
  const server = user
    ? [accountCard(user), devicesCard(), user.admin ? membersCard() : null]
    : window.__TAURI__ && state.session?.mode === "local" ? [connectCard()] : [];
  const draw = () => page.replaceChildren(...content(stats, tiers, server));
  draw();
  page.cleanup = subscribe((kind) => kind === "ai" && draw());
  return page;
}

function content(s, tiers, server) {
  return [
    h("h1", "Profil"),
    h(
      "div.card",
      h("div.hero-stat", icon("fire-fill"), h("div", h("b", String(s.streak)), h("p.muted", s.streak === 1 ? "Tag in Folge" : "Tage in Folge"))),
      h("p.muted.num", `Insgesamt ${s.total_xp} XP aus ${s.lessons} ${s.lessons === 1 ? "Lektion" : "Lektionen"}.`),
    ),
    h("div.card", h("h2", "Letzte 6 Wochen"), calendar(s.active_days)),
    h("div.card", h("h2", "Diese Woche"), week(s.week, s.goal)),
    settings(s),
    ...server.filter(Boolean),
    aiCard(tiers),
  ];
}

function calendar(active) {
  const on = new Set(active);
  const today = new Date();
  today.setHours(12, 0, 0, 0);
  const end = new Date(today);
  end.setDate(end.getDate() + ((7 - ((end.getDay() + 6) % 7) - 1) % 7)); // bis Sonntag dieser Woche
  const start = new Date(end);
  start.setDate(start.getDate() - 41);
  const cells = DOW.map((d) => h("span.dow", d));
  for (let d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
    const key = iso(d);
    const isOn = on.has(key);
    const future = d > today;
    cells.push(
      h(
        `span.day${isOn ? ".on" : ""}${key === iso(today) ? ".today" : ""}`,
        { title: d.toLocaleDateString("de-DE", { weekday: "long", day: "numeric", month: "long" }), style: future ? { opacity: "0.35" } : undefined, "aria-label": `${d.toLocaleDateString("de-DE")}${isOn ? ", gelernt" : ""}` },
        isOn ? icon("fire-fill") : String(d.getDate()),
      ),
    );
  }
  return h("div.calendar", cells);
}

function week(days, goal) {
  const max = Math.max(goal, ...days.map((d) => d.xp), 1);
  const cols = days.map((d) => {
    const wd = DOW[(new Date(`${d.day}T12:00:00`).getDay() + 6) % 7];
    return h(`div.col${d.xp ? "" : ".zero"}`, { title: `${d.xp} XP` }, h("i", { style: { "--h": String(d.xp / max) } }), h("span", wd));
  });
  return h(
    "div",
    h("div.week", { role: "img", "aria-label": `XP der letzten 7 Tage: ${days.map((d) => d.xp).join(", ")}. Tagesziel ${goal} XP.` }, cols, h("span.goal", { style: { bottom: `${24 + 4 + (goal / max) * 100}px` } })),
    h("p.muted", { style: { "margin-top": "10px" } }, `Gestrichelte Linie: dein Tagesziel von ${goal} XP.`),
  );
}

function settings(s) {
  const soundSwitch = h("input.switch", { type: "checkbox", id: "sound", checked: pref.get("sound", "on") === "on" });
  soundSwitch.addEventListener("change", () => pref.set("sound", soundSwitch.checked ? "on" : "off"));
  return h(
    "div.card",
    h("h2", "Einstellungen"),
    h(
      "div.field",
      h("label", "Tagesziel"),
      segmented(
        [10, 20, 30, 50].map((x) => [x, `${x} XP`]),
        s.goal,
        async (xp) => {
          await call("set_goal", { xp });
          await loadStats();
          emit("stats", state.stats);
        },
        "Tagesziel",
      ),
      h("span.hint", "Eine Lektion bringt etwa 15 bis 30 XP."),
    ),
    h("div.row.between", h("label", { for: "sound", style: { "font-weight": "700" } }, "Töne bei Antworten"), soundSwitch),
    h(
      "div.field",
      h("label", "Darstellung"),
      segmented(
        [
          ["system", "System"],
          ["light", "Hell"],
          ["dark", "Dunkel"],
        ],
        pref.get("theme", "system"),
        (mode) => {
          pref.set("theme", mode);
          applyTheme(mode);
        },
        "Darstellung",
      ),
    ),
  );
}

function aiCard(tiers) {
  const ai = state.ai;
  const onServer = state.session?.mode === "server";
  const canPick = !onServer || state.session.user?.admin;
  const line =
    ai?.state === "ready" ? (ai.loaded ? "Bereit und gerade im Speicher." : "Bereit. Ruht, bis du sie brauchst.") : ai?.state === "downloading" ? `Wird geladen: ${gb(ai.done)} von ${gb(ai.total)} GB` : ai?.state === "error" ? "Nicht verfügbar." : "Wird vorbereitet.";
  return h(
    "div.card",
    h("h2", onServer ? "KI auf dem Server" : "KI auf diesem Gerät"),
    h("p", icon("cpu"), " ", line, onServer ? ` ${ai?.label ?? ""}` : ""),
    aiNotice(),
    !canPick ? null : h(
      "div.stack",
      { role: "radiogroup", "aria-label": "Modell" },
      tiers.map((t) =>
        h(
          "label.tier",
          h("input", {
            type: "radio",
            name: "tier",
            checked: ai?.tier === t.id,
            onchange: () => call("ai_set_tier", { tier: t.id }),
          }),
          h("b", t.label),
          h("small", t.id === "kompakt" ? "Für Handys und ältere Rechner. Schneller, etwas einfachere Fragen." : "Für Laptops, PCs und neue iPads. Bessere Fragen und Erklärungen."),
        ),
      ),
    ),
    h(
      "p.muted",
      onServer
        ? "Die KI läuft auf deinem eigenen Server, nichts geht an fremde Dienste."
        : "Die KI läuft komplett auf diesem Gerät. Nach drei Minuten ohne Nutzung gibt sie den Arbeitsspeicher wieder frei. Ein Wechsel lädt das andere Modell einmalig herunter.",
    ),
  );
}

// ---------- Eigener Server: Konto, Geräte, Mitglieder ----------

const rel = new Intl.RelativeTimeFormat("de", { numeric: "auto" });
function ago(ts) {
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 90) return "gerade aktiv";
  if (s < 3600) return rel.format(-Math.round(s / 60), "minute");
  if (s < 86400) return rel.format(-Math.round(s / 3600), "hour");
  return rel.format(-Math.round(s / 86400), "day");
}

/** Link zum Weitergeben: Teilen-Menü auf dem Handy, sonst Kopieren. */
function linkBox(link, hint) {
  const input = h("input.input", { value: link, readonly: true, "aria-label": "Link", onfocus: (e) => e.target.select() });
  const canShare = !!navigator.share;
  const btn = h("button.btn.secondary", { type: "button" }, icon(canShare ? "share-network" : "copy"), canShare ? "Teilen" : "Kopieren");
  btn.addEventListener("click", async () => {
    try {
      if (canShare) await navigator.share({ url: link });
      else {
        await navigator.clipboard.writeText(link);
        toast("Link kopiert.");
      }
    } catch {
      input.select(); // Teilen abgebrochen oder Zwischenablage gesperrt: Link zum Kopieren markieren
    }
  });
  return h("div.stack", { style: { gap: "6px" } }, h("div.link-box", input, btn), hint ? h("span.hint", hint) : null);
}

/** Fehler eines Server-Aufrufs als Toast, statt die Karte kaputt zu lassen. */
const safe = (fn) => async (...a) => {
  try {
    await fn(...a);
  } catch (e) {
    toast(String(e?.message || e));
  }
};

function accountCard(user) {
  const logout = safe(async () => {
    await call("logout");
    location.replace(location.pathname);
  });
  const out = h("div");
  const appLink = state.session.remote
    ? null
    : h(
        "button.btn.ghost",
        {
          type: "button",
          onclick: safe(async () => {
            const r = await call("connect_link");
            out.replaceChildren(linkBox(r.link, "In der PC- oder Handy-App unter Profil → Eigener Server einfügen. Er enthält den Fingerabdruck des Server-Zertifikats."));
          }),
        },
        icon("devices"),
        "Verbindungslink für die App",
      );
  return h(
    "div.card",
    h("h2", "Konto"),
    h("p", icon("user-circle"), " Angemeldet als ", h("b", user.name), user.admin ? h("span.badge", "Admin") : null),
    state.session.remote ? h("p.muted", `Server: ${state.session.remote.url}`) : null,
    h(
      "div.row",
      { style: { "flex-wrap": "wrap", gap: "8px" } },
      h("button.btn.secondary", { type: "button", onclick: changePassword }, icon("lock-simple"), "Passwort ändern"),
      state.session.remote
        ? h("button.btn.secondary", { type: "button", onclick: safe(() => call("server_disconnect").then(() => location.reload())) }, icon("hard-drives"), "Zurück zu lokal")
        : h("button.btn.ghost", { type: "button", onclick: logout }, icon("sign-out"), "Abmelden"),
      appLink,
    ),
    out,
  );
}

function changePassword() {
  const current = h("input.input", { id: "pw-current", type: "password", autocomplete: "current-password" });
  const next = h("input.input", { id: "pw-new", type: "password", autocomplete: "new-password", maxlength: 256, "aria-describedby": "pw-error" });
  const err = h("p.form-error", { id: "pw-error", role: "alert", hidden: true });
  const save = h("button.btn.block", { type: "button" }, "Passwort ändern");
  const s = openSheet({
    title: "Passwort ändern",
    body: [
      h("div.field", h("label", { for: "pw-current" }, "Aktuelles Passwort"), current),
      h("div.field", h("label", { for: "pw-new" }, "Neues Passwort"), next, h("span.hint", PASSWORD_RULES)),
      h("p.muted", "Alle anderen Geräte werden danach abgemeldet."),
      err,
    ],
    foot: save,
  });
  save.addEventListener("click", async () => {
    save.disabled = true;
    try {
      await call("change_password", { current: current.value, password: next.value });
      s.close();
      toast("Passwort geändert. Andere Geräte sind abgemeldet.");
    } catch (e) {
      err.textContent = e?.locked ? "Zu viele Fehlversuche. Versuch es in ein paar Minuten noch einmal." : String(e?.message || e);
      err.hidden = false;
      save.disabled = false;
    }
  });
}

function devicesCard() {
  const list = h("div", h("div.skeleton", { style: { height: "48px" } }));
  const card = h("div.card", h("h2", "Angemeldete Geräte"), list);
  const draw = safe(async () => {
    const all = await call("sessions_list");
    const others = all.filter((x) => !x.current).length;
    list.replaceChildren(
      ...all.map((x) =>
        h(
          "div.list-row",
          h("div", h("b", x.label, x.current ? h("span.badge", "Dieses Gerät") : null), h("span.muted.num", `${ago(x.last_seen)} · ${x.ip}`)),
          x.current ? null : h("button.btn.ghost", { type: "button", onclick: safe(() => call("session_revoke", { id: x.id }).then(draw)) }, "Abmelden"),
        ),
      ),
      ...(others > 1
        ? [h("button.btn.secondary", { type: "button", style: { "margin-top": "8px" }, onclick: safe(() => call("sessions_revoke_others").then(draw)) }, icon("sign-out"), "Alle anderen abmelden")]
        : []),
    );
  });
  draw();
  return card;
}

function membersCard() {
  const list = h("div", h("div.skeleton", { style: { height: "48px" } }));
  const out = h("div.stack");
  const invite = h("button.btn.secondary", { type: "button" }, icon("users"), "Jemanden einladen");
  invite.addEventListener(
    "click",
    safe(async () => {
      const r = await call("invite_create", {});
      out.replaceChildren(linkBox(r.link, "Gilt 7 Tage und nur für ein Konto. Wer den Link hat, kann ein Konto erstellen. Gib ihn nur gezielt weiter."));
    }),
  );

  const draw = safe(async () => {
    const all = await call("members");
    list.replaceChildren(
      ...all.map((m) => {
        const badges = [m.admin ? h("span.badge", "Admin") : null, m.disabled ? h("span.badge.off", "Gesperrt") : null, m.me ? h("span.badge", "Du") : null];
        const actions = m.me
          ? null
          : h(
              "div.row",
              { style: { gap: "0", "flex-wrap": "wrap", "justify-content": "end" } },
              h("button.btn.ghost", { type: "button", onclick: safe(() => passwordLink(m)) }, "Passwort-Link"),
              h("button.btn.ghost", { type: "button", style: m.disabled ? null : { color: "var(--wrong)" }, onclick: safe(() => toggleMember(m)) }, m.disabled ? "Entsperren" : "Sperren"),
            );
        return h("div.list-row", h("div", h("b", m.name, ...badges)), actions);
      }),
    );
  });

  async function passwordLink(m) {
    const r = await call("invite_create", { userId: m.id });
    out.replaceChildren(linkBox(r.link, `Damit setzt ${m.name} ein neues Passwort. Gilt 7 Tage, alle Geräte werden dabei abgemeldet.`));
  }
  async function toggleMember(m) {
    if (!m.disabled) {
      const yes = await confirmSheet({
        title: `${m.name} sperren?`,
        text: "Die Person wird auf allen Geräten sofort abgemeldet und kann sich nicht mehr anmelden. Kurse und Fortschritt bleiben erhalten, Entsperren geht jederzeit.",
        confirm: "Sperren",
        danger: true,
      });
      if (!yes) return;
    }
    await call("member_disable", { id: m.id, disabled: !m.disabled });
    draw();
  }

  draw();
  return h("div.card", h("h2", "Mitglieder"), h("p.muted", "Nur wer hier ein Konto hat, kommt auf diesen Server."), list, invite, out);
}

// ---------- App: mit eigenem Server verbinden (lokal bleibt immer verfügbar) ----------

function connectCard() {
  const link = h("input.input", { id: "srv-link", autocomplete: "off", spellcheck: "false", placeholder: "lernpfad://…" });
  const name = h("input.input", { id: "srv-name", autocomplete: "username", autocapitalize: "none" });
  const password = h("input.input", { id: "srv-pw", type: "password", autocomplete: "current-password" });
  const err = h("p.form-error", { role: "alert", hidden: true });
  const go = h("button.btn.secondary", { type: "button" }, icon("hard-drives"), "Verbinden");
  go.addEventListener("click", async () => {
    err.hidden = true;
    go.disabled = true;
    try {
      await call("server_connect", { link: link.value.trim(), name: name.value.trim(), password: password.value });
      location.reload();
    } catch (e) {
      err.textContent = e?.locked ? "Zu viele Fehlversuche. Versuch es in ein paar Minuten noch einmal." : String(e?.message || e);
      err.hidden = false;
      go.disabled = false;
    }
  });
  return h(
    "details.card.q-box",
    h("summary", h("h2", "Eigener Server"), icon("caret-down")),
    h("p.muted", "Mit deinem Lernpfad-Server lernen, z. B. zusammen mit Kommilitonen. Deine Daten auf diesem Gerät bleiben unberührt, „Zurück zu lokal“ geht jederzeit."),
    h("div.field", h("label", { for: "srv-link" }, "Verbindungslink"), link, h("span.hint", "Bekommst du vom Admin des Servers (Profil → Mitglieder).")),
    h("div.field", h("label", { for: "srv-name" }, "Name"), name),
    h("div.field", h("label", { for: "srv-pw" }, "Passwort"), password),
    err,
    go,
  );
}
