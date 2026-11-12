// Brücke zum Kern. Drei Wege, dieselben Befehle:
//  - Tauri-App: invoke("rpc") → Rust (lokal oder, wenn verbunden, an den eigenen Server weitergeleitet)
//  - Web-App vom eigenen Server: fetch("api/rpc/<cmd>"), Ereignisse per Server-Sent Events
//  - Entwicklung im Browser: dev/mock-tauri.js setzt ein nachgebautes __TAURI__
const T = () => window.__TAURI__;

/** Server sagt 401: Sitzung abgelaufen oder abgemeldet → main.js zeigt die Anmeldung. */
const loggedOut = () => dispatchEvent(new Event("lernpfad:logged-out"));

async function http(cmd, args, onPiece) {
  let res;
  try {
    res = await fetch(`api/rpc/${cmd}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args),
      credentials: "same-origin",
    });
  } catch {
    throw "Der Server ist gerade nicht erreichbar.";
  }
  if (res.status === 401) {
    loggedOut();
    throw "Bitte melde dich an.";
  }
  if (res.status === 429) {
    const e = await res.json().catch(() => ({}));
    throw { locked: Number(e.retry_after || res.headers.get("retry-after") || 30), message: e.error || "Zu viele Versuche." };
  }
  if (!res.ok) throw (await res.text()) || `Fehler ${res.status}`;
  if (!onPiece) return res.json();

  // NDJSON: {"t": Stück} … dann {"ok": Ergebnis} oder {"err": Meldung}
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) throw "Die Verbindung zum Server ist abgebrochen.";
    buf += value;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i);
      buf = buf.slice(i + 1);
      if (!line) continue;
      const m = JSON.parse(line);
      if ("t" in m) onPiece(m.t);
      else if ("ok" in m) return m.ok;
      else throw m.err;
    }
  }
}

/** Tauri: Fehler kommen als Text, bei Server-Verbindung auch als { auth } (Sitzung weg) oder { locked } (Sperre). */
function tauri(name, payload) {
  return T().core.invoke(name, payload).catch((e) => {
    if (e?.auth) {
      loggedOut();
      throw e.message;
    }
    throw e;
  });
}

export function call(cmd, args = {}) {
  return T() ? tauri("rpc", { cmd, args }) : http(cmd, args);
}

/** Befehl mit gestreamter Antwort: `onPiece` bekommt den Text stückweise. */
export function stream(cmd, args, onPiece) {
  if (!T()) return http(cmd, args, onPiece);
  const ch = new (T().core.Channel)();
  ch.onmessage = onPiece;
  return tauri("rpc_stream", { cmd, args, onToken: ch });
}

let events;
export function on(event, fn) {
  if (T()) return T().event.listen(event, (e) => fn(e.payload));
  events ??= new EventSource("api/events");
  events.addEventListener(event, (e) => fn(JSON.parse(e.data)));
}
