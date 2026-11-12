// Brücke zum Rust-Kern (Tauri). Im Browser ersetzt dev/mock-tauri.js das globale __TAURI__.
const T = () => window.__TAURI__;

export const call = (cmd, args = {}) => T().core.invoke(cmd, args);

export const on = (event, fn) => T().event.listen(event, (e) => fn(e.payload));

/** Befehl mit gestreamter Antwort: `onPiece` bekommt den Text stückweise. */
export function stream(cmd, args, onPiece) {
  const ch = new (T().core.Channel)();
  ch.onmessage = onPiece;
  return call(cmd, { ...args, onToken: ch });
}
