// The pictures that stand in for written instructions. They're built as SVG
// nodes and styled and animated from style.css, because the page allows no
// inline styles.

const SVG = "http://www.w3.org/2000/svg";

function s(tag: string, attrs: Record<string, string | number> = {}, ...children: (Node | string)[]): SVGElement {
  const element = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attrs)) element.setAttribute(name, String(value));
  element.append(...children);
  return element;
}

function picture(name: string, width: number, height: number, label: string, ...children: Node[]): SVGElement {
  return s(
    "svg",
    { class: `diagram ${name}`, viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": label },
    ...children,
  );
}

/** The keypad's backspace key, centred on (x, y) and `size` tall. */
function backspace(x: number, y: number, size: number): SVGElement {
  const half = size / 2;
  const arm = size * 0.18;
  const cross = x + size * 0.2;
  return s("path", {
    class: "glyph",
    d:
      `M${x - size * 0.75} ${y}L${x - size * 0.3} ${y - half}H${x + size * 0.75}V${y + half}H${x - size * 0.3}Z` +
      `M${cross - arm} ${y - arm}l${arm * 2} ${arm * 2}M${cross + arm} ${y - arm}l${-arm * 2} ${arm * 2}`,
  });
}

const KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "", "0", "DELETE"];

/** A phone's number pad, with its top left corner at (x, y). */
function keypad(x: number, y: number, width: number, height: number, gap: number, pressed?: string): SVGElement {
  const pad = s("g");
  KEYS.forEach((key, index) => {
    if (!key) return;
    const left = x + (index % 3) * (width + gap);
    const top = y + Math.floor(index / 3) * (height + gap);
    const [midX, midY] = [left + width / 2, top + height / 2];
    pad.append(
      s(
        "g",
        { class: key === pressed ? "key pressed" : "key" },
        s("rect", { x: left, y: top, width, height, rx: 5 }),
        key === "DELETE" ? backspace(midX, midY, height * 0.42) : s("text", { x: midX, y: midY }, key),
      ),
    );
  });
  return pad;
}

/** A small bin, for the way into Recently deleted. It's decoration: the
 * button it sits in carries the words. */
export function binIcon(): SVGElement {
  return s(
    "svg",
    { class: "icon", viewBox: "0 0 16 16", "aria-hidden": "true" },
    s("path", { d: "M2.5 4.5h11M6.5 4.5v-2h3v2M3.8 4.5l.7 9h7l.7-9M6.7 7.2v3.6M9.3 7.2v3.6" }),
  );
}

/** A small cog, for the way into Settings: eight teeth around a ring, with
 * a hole in the middle. */
export function cogIcon(): SVGElement {
  const outline: string[] = [];
  for (let tooth = 0; tooth < 8; tooth++) {
    const middle = (tooth * Math.PI) / 4;
    // each tooth rises from the ring, runs flat, and comes back down
    for (const [turn, radius] of [
      [-0.27, 5],
      [-0.17, 6.8],
      [0.17, 6.8],
      [0.27, 5],
    ]) {
      const angle = middle + turn;
      outline.push(`${(8 + radius * Math.cos(angle)).toFixed(2)} ${(8 + radius * Math.sin(angle)).toFixed(2)}`);
    }
  }
  return s(
    "svg",
    { class: "icon", viewBox: "0 0 16 16", "aria-hidden": "true" },
    s("path", { d: `M${outline.join("L")}Z` }),
    s("circle", { cx: 8, cy: 8, r: 2.2 }),
  );
}

/** A small heart, for Donate. */
export function heartIcon(): SVGElement {
  return s(
    "svg",
    { class: "icon", viewBox: "0 0 16 16", "aria-hidden": "true" },
    s("path", {
      d: "M13.9 3.1a3.67 3.67 0 0 0-5.2 0l-.7.7-.7-.7a3.67 3.67 0 0 0-5.2 5.2l.7.7L8 14.2 13.2 9l.7-.7a3.67 3.67 0 0 0 0-5.2Z",
    }),
  );
}

/** The backspace key by itself, for wherever DELETE is named. */
export function backspaceIcon(): SVGElement {
  return picture("backspace", 36, 24, "the backspace key", backspace(18, 12, 16));
}

/** The phone on its passcode screen. With `covered`, a sticky note comes down
 * over the row of dots. */
export function phone(covered: boolean): SVGElement {
  const dots = [54, 78, 102, 126].map((cx) => s("circle", { class: "dot", cx, cy: 132, r: 6.5 }));
  const note = s(
    "g",
    { class: "note" },
    s("rect", { class: "note-paper", x: 34, y: 108, width: 112, height: 48, rx: 2 }),
    s("rect", { class: "note-strip", x: 34, y: 108, width: 112, height: 11, rx: 2 }),
  );
  return picture(
    "phone",
    180,
    360,
    covered ? "A sticky note covering the row of dots above the keypad" : "The passcode screen: a row of dots above the keypad",
    s("rect", { class: "phone-body", x: 3, y: 3, width: 174, height: 354, rx: 30 }),
    s("rect", { class: "phone-island", x: 68, y: 15, width: 44, height: 12, rx: 6 }),
    s("rect", { class: "phone-line", x: 42, y: 78, width: 96, height: 8, rx: 4 }),
    ...dots,
    keypad(15, 204, 46, 30, 6),
    ...(covered ? [note] : []),
  );
}

/** How one tap goes: the app names a key, you tap it on the phone, then you
 * press Space. Each part lights up in turn. */
export function tapLoop(): SVGElement {
  const part = (x: number, turn: number, caption: string, ...inside: Node[]) =>
    s(
      "g",
      { class: `turn turn-${turn}` },
      s("rect", { class: "tile", x, y: 1, width: 158, height: 118, rx: 8 }),
      ...inside,
      s("text", { class: "caption", x: x + 79, y: 144 }, caption),
    );
  const arrow = (x: number) => s("path", { class: "arrow", d: `M${x} 60h22m-7 -7l7 7l-7 7` });
  return picture(
    "loop",
    560,
    160,
    "The app shows a key. Tap it on your iPhone, then press Space.",
    part(
      1,
      1,
      "The app shows a key",
      s(
        "g",
        { class: "shown" },
        s("text", { class: "loop-word", x: 56, y: 63 }, "TAP"),
        s("text", { class: "loop-key", x: 106, y: 60 }, "7"),
      ),
    ),
    arrow(169),
    // the ripple sits on the pad's 7 key
    part(201, 2, "Tap it on your iPhone", keypad(221, 12.5, 36, 20, 5, "7"), s("circle", { class: "ripple", cx: 239, cy: 72.5, r: 17 })),
    arrow(369),
    part(
      401,
      3,
      "Press Space",
      s(
        "g",
        { class: "spacebar" },
        s("rect", { class: "keycap", x: 425, y: 42, width: 110, height: 36, rx: 7 }),
        s("text", { class: "keycap-text", x: 480, y: 60 }, "Space"),
      ),
    ),
  );
}
