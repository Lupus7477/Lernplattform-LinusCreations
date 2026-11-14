// Lern-Erinnerung (nur in den Apps). Handy: geplante Benachrichtigungen für die nächsten 30 Tage
// (neu geplant bei jedem Start und jeder Lektion; wer länger weg ist, bekommt trotzdem weiter eine).
// PC: Timer, solange die App läuft (das Plugin plant dort nicht, es würde sofort melden).
// Ist das Tagesziel heute erreicht, fällt die heutige Erinnerung weg.
import { call } from "./api.js";
import { pref } from "./ui.js";

const N = () => window.__TAURI__?.notification;
const mobile = () => /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);
const IDS = Array.from({ length: 30 }, (_, d) => 7701 + d);
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

/** `d` = in wie vielen Tagen. Die Zahl der fälligen Fragen stimmt nur für heute und morgen, später ändert sie sich. */
function text(stats, d = 0) {
  const streak = stats?.streak || 0;
  const due = d <= 1 ? stats?.due || 0 : 0;
  return {
    title: "Zeit für eine Lektion",
    body: due
      ? `${due} ${due === 1 ? "Frage wartet" : "Fragen warten"} auf die Wiederholung. Ein paar Minuten reichen.`
      : streak > 1 && d <= 1
        ? `Dein ${streak}-Tage-Streak wartet. Eine Lektion dauert nur ein paar Minuten.`
        : "Ein paar Minuten reichen schon, um dranzubleiben.",
  };
}

/** Termine der nächsten 30 Tage zur gewählten Uhrzeit, heute nur, wenn noch offen und das Ziel nicht erreicht ist. */
function upcoming(time, stats) {
  const [hh, mm] = time.split(":").map(Number);
  const done = (stats?.xp_today || 0) >= (stats?.goal || 1);
  const now = new Date();
  return IDS.map((id, d) => {
    const at = new Date(now);
    at.setDate(now.getDate() + d);
    at.setHours(hh, mm, 0, 0);
    return { id, at, d };
  }).filter(({ at, d }) => at > now && !(d === 0 && done));
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
      for (const { id, at, d } of upcoming(time, stats)) n.sendNotification({ id, ...text(stats, d), schedule: n.Schedule.at(at, false, true) });
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
