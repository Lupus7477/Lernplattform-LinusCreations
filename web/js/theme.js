// Gewähltes Theme vor dem ersten Zeichnen setzen (kein helles Aufblitzen im Dunkelmodus).
try {
  const t = localStorage.getItem("theme");
  if (t === "light" || t === "dark") document.documentElement.dataset.theme = t;
} catch {}
