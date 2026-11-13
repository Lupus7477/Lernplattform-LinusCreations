// Lern-Erinnerung (nur in den Apps). Handy: geplante Benachrichtigungen für die nächsten 7 Tage.
// PC: Timer, solange die App läuft (das Plugin plant dort nicht, es würde sofort melden).
// Ist das Tagesziel heute erreicht, fällt die heutige Erinnerung weg.
import { call } from "./api.js";
import { pref } from "./ui.js";

const N = () => window.__TAURI__?.notification;
const mobile = () => /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);
const IDS = [7701, 7702, 7703, 7704, 7705, 7706, 7707];
let timer = null;

/** Erinnerung möglich? (App mit Notification-Plugin) */
export const canRemind = () => !!N();

/** Gewählte Uhrzeit „HH:MM“ oder "" (aus). */
export const reminderTime = () => pref.get("remind", "");

/** Einschalten fragt nach der Berechtigung. Gibt zurück, ob die Erinnerung jetzt an ist. */
export async function setReminder(time, stats) {
  const n = N();
  if (time && n) {
    const ok = (await n.isPermissionGranted()) || (await n.requestPermission()) === "granted";
    if (!ok) time = "";
  }
  pref.set("remind", time || "");
  await syncReminder(stats);
  return !!time;
}

function text(stats) {
  const streak = stats?.streak || 0;
  return {
    title: "Zeit für eine Lektion",
    body: streak > 1 ? `Dein ${streak}-Tage-Streak wartet. Eine Lektion dauert nur ein paar Minuten.` : "Ein paar Minuten reichen schon, um dranzubleiben.",
  };
}

/** Termine der nächsten 7 Tage zur gewählten Uhrzeit, heute nur, wenn noch offen und das Ziel nicht erreicht ist. */
function upcoming(time, stats) {
  const [hh, mm] = time.split(":").map(Number);
  const done = (stats?.xp_today || 0) >= (stats?.goal || 1);
  const now = new Date();
  return IDS.map((id, d) => {
    const at = new Date(now);
    at.setDate(now.getDate() + d);
    at.setHours(hh, mm, 0, 0);
    return { id, at };
  }).filter(({ at }, d) => at > now && !(d === 0 && done));
}

/** Nach jedem Start und jeder Lektion: Erinnerungen neu planen. */
export async function syncReminder(stats) {
  const n = N();
  if (!n) return;
  clearTimeout(timer);
  const time = reminderTime();
  try {
    if (mobile()) {
      await n.cancel(IDS);
      if (!time) return;
      const t = text(stats);
      for (const { id, at } of upcoming(time, stats)) n.sendNotification({ id, ...t, schedule: n.Schedule.at(at, false, true) });
      return;
    }
    const next = time && upcoming(time, stats)[0];
    if (!next) return;
    timer = setTimeout(async () => {
      const s = await call("stats").catch(() => stats);
      if ((s?.xp_today || 0) < (s?.goal || 1)) n.sendNotification(text(s));
      syncReminder({ ...s, xp_today: s?.goal || 1 }); // heute erledigt → morgen wieder
    }, Math.min(next.at - Date.now(), 2 ** 31 - 1));
  } catch (e) {
    console.warn("Erinnerung", e);
  }
}
