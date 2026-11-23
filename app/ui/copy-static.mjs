// Copies what tsc doesn't produce into dist, beside the compiled scripts.
import { copyFileSync } from "node:fs";

const here = new URL(".", import.meta.url);
for (const [from, to] of [
  ["index.html", "index.html"],
  ["style.css", "style.css"],
  ["../../assets/icon/icon.svg", "icon.svg"],
]) {
  copyFileSync(new URL(from, here), new URL(`dist/${to}`, here));
}
