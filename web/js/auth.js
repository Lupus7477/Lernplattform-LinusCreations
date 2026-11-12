// Anmeldung am eigenen Server: Anmelden, Einrichten (erstes Konto), Beitreten (Einladung), neues Passwort.
// Ohne gültige Sitzung zeigt die App nur diese Seite; alle Daten liefert der Server erst nach der Anmeldung.
import { call } from "./api.js";
import { h, icon, shake } from "./ui.js";

export const PASSWORD_RULES = "Mindestens 12 Zeichen, am besten ein Satz aus mehreren Wörtern. Bekannte, geleakte Passwörter werden abgelehnt.";

export function show(session) {
  const [route, code] = location.hash.replace(/^#\/?/, "").split("/");
  const views = {
    setup: () => newPassword("setup", code),
    join: () => newPassword("join", code),
    reset: () => newPassword("reset", code),
  };
  const view = code && views[route] ? views[route]() : login(session);
  document.getElementById("app").replaceChildren(h("main.auth", { id: "main" }, view));
  view.querySelector("input")?.focus();
}

/** Nach Erfolg: Code aus der Adresse entfernen (nicht im Verlauf lassen) und die App laden. */
function done() {
  location.replace(location.pathname);
}

function field(id, label, input, hint) {
  input.id = id;
  return h("div.field", h("label", { for: id }, label), input, hint ? h("span.hint", hint) : null);
}

function passwordInput(autocomplete) {
  return h("input.input", { type: "password", autocomplete, required: true, maxlength: 256, "aria-describedby": "auth-error" });
}

function showToggle(...inputs) {
  const box = h("input", { type: "checkbox", id: "auth-show" });
  box.addEventListener("change", () => inputs.forEach((i) => (i.type = box.checked ? "text" : "password")));
  return h("label.row", { for: "auth-show", style: { gap: "8px", "font-weight": "600" } }, box, "Passwort anzeigen");
}

function form({ title, text, fields, submit, onSubmit }) {
  const err = h("p.form-error", { id: "auth-error", role: "alert", hidden: true });
  const btn = h("button.btn.block.num", { type: "submit" }, submit);
  const card = h("form.card", h("div.auth-brand", icon("path"), "Lernpfad"), h("h1", title), text ? h("p.muted", text) : null, ...fields, err, btn);
  let timer;

  // Sperre vom Server: Knopf zählt herunter, ohne Animation (Statushinweis, kein Effekt).
  function lockout(secs) {
    clearInterval(timer);
    err.textContent = "Zu viele Fehlversuche. Zum Schutz deines Kontos ist die Anmeldung kurz gesperrt.";
    err.hidden = false;
    btn.disabled = true;
    const end = Date.now() + secs * 1000;
    const tick = () => {
      const left = Math.max(0, Math.ceil((end - Date.now()) / 1000));
      btn.textContent = `Erneut in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
      if (left === 0) {
        clearInterval(timer);
        btn.textContent = submit;
        btn.disabled = false;
        err.hidden = true;
      }
    };
    tick();
    timer = setInterval(tick, 1000);
  }

  card.addEventListener("submit", async (e) => {
    e.preventDefault();
    if (btn.disabled) return;
    err.hidden = true;
    btn.disabled = true;
    try {
      await onSubmit();
      done();
    } catch (x) {
      if (x?.locked) return lockout(x.locked);
      err.textContent = String(x?.message || x);
      err.hidden = false;
      btn.disabled = false;
      shake(card);
    }
  });
  return card;
}

function login(session) {
  const name = h("input.input", { autocomplete: "username", required: true, maxlength: 64, autocapitalize: "none", spellcheck: "false" });
  const password = passwordInput("current-password");
  return form({
    title: "Anmelden",
    text: session.setup
      ? "Dieser Server ist noch nicht eingerichtet. Öffne den Einrichtungslink, den der Server beim Start ins Protokoll schreibt."
      : "Dieser Lernpfad-Server ist privat. Melde dich mit deinem Konto an.",
    fields: [field("auth-name", "Name", name), field("auth-password", "Passwort", password), showToggle(password)],
    submit: "Anmelden",
    onSubmit: () => call("login", { name: name.value.trim(), password: password.value }),
  });
}

function newPassword(kind, code) {
  const name = h("input.input", { autocomplete: "username", required: true, maxlength: 64, autocapitalize: "none", spellcheck: "false" });
  const password = passwordInput("new-password");
  const copy = {
    setup: ["Server einrichten", "Du legst das erste Konto an. Es verwaltet den Server und lädt andere ein.", "Konto anlegen"],
    join: ["Konto erstellen", "Du wurdest eingeladen. Wähle einen Namen und ein Passwort.", "Konto erstellen"],
    reset: ["Neues Passwort", "Lege ein neues Passwort fest. Dabei werden alle Geräte abgemeldet.", "Passwort speichern"],
  }[kind];
  const fields = [
    kind === "reset" ? null : field("auth-name", "Name", name, "So sehen dich andere auf diesem Server."),
    field("auth-password", "Passwort", password, PASSWORD_RULES),
    showToggle(password),
  ].filter(Boolean);
  return form({
    title: copy[0],
    text: copy[1],
    fields,
    submit: copy[2],
    onSubmit: () => call(kind === "reset" ? "reset_password" : kind, { code, name: name.value.trim(), password: password.value }),
  });
}
