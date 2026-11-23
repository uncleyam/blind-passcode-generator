// Puts the appearance in place before anything is drawn. The page loads this
// as an ordinary script in its head, ahead of the stylesheet, so a window
// never opens in one set of colours and then flips to the other.
//
// Settings, in main.ts, keeps "light", "dark" or "system" in the page's own
// storage under "appearance". Nothing saved means dark.
(() => {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem("appearance");
  } catch {
    // no storage to read: dark, as on a first run
  }
  // with no theme set at all, the stylesheet follows the system
  if (saved !== "system") document.documentElement.dataset.theme = saved === "light" ? "light" : "dark";
})();
