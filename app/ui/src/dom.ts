// A small DOM builder and the keyboard plumbing. Text always goes in as
// text, never as HTML, because labels and paths come from files.

export type Child = Node | string | false | null | undefined;
type Props = Record<string, string | number | boolean | undefined | ((event: Event) => void)>;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Props = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  for (const [name, value] of Object.entries(props)) {
    if (value === undefined || value === false) continue;
    if (typeof value === "function") {
      element.addEventListener(name.slice(2).toLowerCase(), value);
    } else if (name === "class") {
      element.className = String(value);
    } else {
      element.setAttribute(name, value === true ? "" : String(value));
    }
  }
  for (const child of children) {
    if (child !== false && child !== null && child !== undefined) element.append(child);
  }
  return element;
}

/** A button that gives focus back after a click, so a later Space or Enter
 * can't press it a second time. "reveal" and "delete" are tinted, so the two
 * are told apart at a glance; "accent" and "danger" are their solid forms,
 * for the button that goes through with it. */
export function button(
  text: string,
  onClick: () => void,
  options: {
    kind?: "primary" | "quiet" | "reveal" | "delete" | "accent" | "danger";
    hint?: string;
    /** A small picture before the words. */
    icon?: Node;
    /** What a button with no words is called, for a tooltip and for
     * screen readers. */
    label?: string;
    disabled?: boolean;
  } = {},
): HTMLButtonElement {
  const element = h(
    "button",
    {
      type: "button",
      class: [options.kind, text ? "" : "icon-only"].filter(Boolean).join(" "),
      title: options.label,
      "aria-label": options.label,
      disabled: options.disabled,
      onClick: (event: Event) => {
        (event.currentTarget as HTMLElement).blur();
        onClick();
      },
    },
    options.icon,
    text,
    options.hint ? h("kbd", {}, options.hint) : null,
  );
  return element;
}

/** Handles a key on the current screen; returns true if it was used. */
export type KeyHandler = (key: string) => boolean;

let currentKeys: KeyHandler | null = null;

/** Replaces the whole screen. Nothing of the previous screen survives. A
 * "focus" screen sits a little above the middle of the window; a "list"
 * starts at the top. */
export function show(nodes: Child[], keys: KeyHandler | null = null, layout: "focus" | "list" = "focus"): void {
  const app = document.getElementById("app")!;
  app.className = layout;
  app.replaceChildren(...nodes.filter((node): node is Node | string => Boolean(node)));
  currentKeys = keys;
  const wanted = app.querySelector<HTMLElement>("[autofocus]");
  (wanted ?? app).focus();
  app.scrollTop = 0;
}

const actionKeys = new Set([" ", "Enter"]);

document.addEventListener("keydown", (event) => {
  // reloading, printing or searching would only get in the way
  const key = event.key.toLowerCase();
  if (event.key === "F5" || ((event.ctrlKey || event.metaKey) && "rpfgu".includes(key))) {
    event.preventDefault();
    return;
  }
  if (!currentKeys || event.target instanceof HTMLInputElement) return;
  // a held key must never run through instructions
  if (event.repeat) {
    event.preventDefault();
    return;
  }
  if (currentKeys(event.key)) event.preventDefault();
});

// a focused button would otherwise act again when the key comes up
document.addEventListener("keyup", (event) => {
  if (currentKeys && actionKeys.has(event.key) && !(event.target instanceof HTMLInputElement)) {
    event.preventDefault();
  }
});

document.addEventListener("contextmenu", (event) => event.preventDefault());

// The title bar. Pressing on its left stretch drags the window; the buttons
// minimize and close it.
function windowCommand(command: string): void {
  window.__TAURI__.core.invoke(command).catch(() => {});
}

document.getElementById("titlebar-drag")?.addEventListener("mousedown", (event) => {
  if (event.button === 0) windowCommand("start_dragging");
});
for (const [id, command] of [
  ["minimize", "minimize_window"],
  ["close", "close_window"],
]) {
  document.getElementById(id)?.addEventListener("click", (event) => {
    (event.currentTarget as HTMLElement).blur();
    windowCommand(command);
  });
}
