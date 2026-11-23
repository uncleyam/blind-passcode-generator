// Locker's screens. During a setup the Rust side holds the code and hands
// out one instruction at a time; this page only shows what it's given.
//
// Screens say as little as they can. Where a picture can carry an
// instruction, it does, and a long explanation becomes another screen.

import {
  api,
  type DeletedCode,
  type DeleteInfo,
  type DeleteOutcome,
  type Link,
  type Overview,
  type RevealEvent,
  type RevealInfo,
  type SetupAction,
  type SetupView,
} from "./api.js";
import { backspaceIcon, binIcon, cogIcon, heartIcon, phone, tapLoop } from "./diagrams.js";
import { button, h, show, type Child, type KeyHandler } from "./dom.js";

/** After each setup screen appears, input waits this long. Every tap gets the
 * same pause, and it stops a double press from skipping an instruction. */
const INPUT_PAUSE_MS = 300;

/** The memory check's answer ignores Enter for this long after it appears. */
const RESULT_PAUSE_MS = 600;

type Finished = Extract<SetupView, { kind: "finished" }>;

function message(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

function actions(...buttons: HTMLElement[]): HTMLDivElement {
  return h("div", { class: "actions" }, ...buttons);
}

function centredActions(...buttons: HTMLElement[]): HTMLDivElement {
  return h("div", { class: "actions centred" }, ...buttons);
}

/** Words on the left, a picture on the right. */
function beside(picture: SVGElement, ...words: Child[]): HTMLDivElement {
  return h("div", { class: "split" }, h("div", {}, ...words), picture);
}

function onKey(wanted: string, run: () => void): KeyHandler {
  return (key) => {
    if (key !== wanted) return false;
    run();
    return true;
  };
}

/** Mirrors the Rust side's wording for a duration. */
function fmtDuration(seconds: number): string {
  if (seconds < 90) {
    const n = Math.max(1, Math.round(seconds));
    return `${n} second${n === 1 ? "" : "s"}`;
  }
  if (seconds < 5400) return `${Math.round(seconds / 60)} minutes`;
  return `${(seconds / 3600).toFixed(1)} hours`;
}

function failure(error: unknown): void {
  show([
    h("h1", {}, "Something went wrong"),
    h("p", {}, message(error)),
    actions(button("Home", () => void home(), { kind: "primary" })),
  ]);
}

/** A field that must hold one word before its button works. Only what can't
 * be undone asks for this. */
function typedConfirmation(
  word: string,
  buttonText: string,
  onConfirm: (typed: string) => void,
  onCancel: () => void,
): HTMLFormElement {
  const typed = h("input", {
    id: "confirm",
    autocomplete: "off",
    spellcheck: "false",
    autofocus: true,
    placeholder: word,
  });
  const go = h("button", { type: "submit", class: "danger", disabled: true }, buttonText);
  typed.addEventListener("input", () => {
    go.disabled = typed.value.trim().toLowerCase() !== word;
  });
  return h(
    "form",
    {
      onSubmit: (event: Event) => {
        event.preventDefault();
        if (!go.disabled) onConfirm(typed.value);
      },
    },
    h("label", { for: "confirm" }, `Type ${word}`),
    typed,
    actions(go, button("Cancel", onCancel, { kind: "quiet" })),
  );
}

// ------------------------------------------------------------------ home

async function home(notice?: string): Promise<void> {
  let overview: Overview;
  try {
    overview = await api.overview();
  } catch (error) {
    failure(error);
    return;
  }

  const unfinished =
    overview.unfinished.length > 0 &&
    h(
      "section",
      { class: "card warning" },
      h("h2", {}, "Unfinished setup"),
      h("p", {}, "Your phone may have this code. Keep it unless you're sure it doesn't."),
      ...overview.unfinished.map((item) =>
        h(
          "div",
          { class: "row" },
          h(
            "div",
            { class: "row-main" },
            h("div", { class: "row-title" }, item.label),
            h("div", { class: "row-meta" }, item.created),
          ),
          h(
            "div",
            { class: "row-actions" },
            button("Keep", () => void keepUnfinished(item.path), { kind: "primary" }),
            button("Discard", () => discardUnfinished(item.path, item.label), { kind: "delete" }),
          ),
        ),
      ),
    );

  const codes =
    overview.codes.length > 0
      ? h(
          "ul",
          { class: "codes" },
          ...overview.codes.map((code) =>
            h(
              "li",
              { class: "row", title: code.path },
              h(
                "div",
                { class: "row-main" },
                h("div", { class: "row-title" }, code.label),
                h(
                  "div",
                  { class: "row-meta" },
                  [code.created, code.cost, code.backupOnly ? "backup only" : ""].filter(Boolean).join(" · "),
                ),
              ),
              h(
                "div",
                { class: "row-actions" },
                button("Reveal", () => void revealScreen(code.path), { kind: "reveal" }),
                button("Delete", () => void deleteScreen(code.path), { kind: "delete" }),
              ),
            ),
          ),
        )
      : h("p", { class: "empty" }, "Set a passcode that you yourself don't know.");

  show(
    [
      h(
        "div",
        { class: "head" },
        h("h1", {}, "Saved codes"),
        h(
          "div",
          { class: "head-actions" },
          button("", () => void settingsScreen(), { kind: "quiet", icon: cogIcon(), label: "Settings" }),
          lockTextShown && button("Lock text", lockTextScreen),
          button("New code", intro, { kind: "primary" }),
        ),
      ),
      notice && h("p", { class: "notice", role: "status" }, notice),
      unfinished,
      codes,
      overview.deleted > 0 &&
        actions(button("Recently deleted", () => void binScreen(), { kind: "quiet", icon: binIcon() })),
      h("p", { class: "footnote" }, overview.folder, overview.backup ? ` · backups in ${overview.backup}` : ""),
    ],
    null,
    "list",
  );
}

async function keepUnfinished(path: string): Promise<void> {
  try {
    await api.keepUnfinished(path);
    await home("Kept.");
  } catch (error) {
    failure(error);
  }
}

function discardUnfinished(path: string, label: string): void {
  show([
    h("h1", {}, `Discard “${label}”?`),
    h("p", { class: "row-path" }, path),
    h("p", { class: "callout danger" }, "If your phone took this code, it's gone for good."),
    typedConfirmation(
      "delete",
      "Discard",
      async (typed) => {
        try {
          await api.discardUnfinished(path, typed);
          await home("Discarded.");
        } catch (error) {
          failure(error);
        }
      },
      () => void home(),
    ),
  ]);
}

// -------------------------------------------------------------- settings

type Appearance = "light" | "dark" | "system";

/** What theme.ts reads when the window opens. Nothing saved means dark. */
const APPEARANCE_KEY = "appearance";

function savedAppearance(): Appearance {
  const set = document.documentElement.dataset.theme;
  return set === "light" || set === "dark" ? set : "system";
}

function setAppearance(appearance: Appearance): void {
  if (appearance === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = appearance;
  try {
    localStorage.setItem(APPEARANCE_KEY, appearance);
  } catch {
    // nowhere to keep it: the choice lasts until the window closes
  }
}

/** Set to "on" when Settings turns Lock text on. It's an extra beside the
 * app's main job, so nothing saved means Home doesn't offer it. */
const LOCK_TEXT_KEY = "lock-text";

function savedLockText(): boolean {
  try {
    return localStorage.getItem(LOCK_TEXT_KEY) === "on";
  } catch {
    return false;
  }
}

/** Whether Home has a Lock text button. Text locked earlier stays on Home
 * either way. */
let lockTextShown = savedLockText();

function setLockText(shown: boolean): void {
  lockTextShown = shown;
  try {
    if (shown) localStorage.setItem(LOCK_TEXT_KEY, "on");
    else localStorage.removeItem(LOCK_TEXT_KEY);
  } catch {
    // nowhere to keep it: the choice lasts until the window closes
  }
}

/** Hands one of the app's own links to the browser or the mail app. */
function openLink(link: Link): void {
  api.openLink(link).catch(failure);
}

async function settingsScreen(): Promise<void> {
  // Settings is worth opening even if the links can't be had
  const links = await api.links().catch(() => null);
  const current = savedAppearance();
  const choice = (appearance: Appearance, text: string) => {
    const input = h("input", { type: "radio", name: "appearance", checked: appearance === current });
    input.addEventListener("change", () => setAppearance(appearance));
    return h("label", { class: "choice" }, input, text);
  };
  const lockText = h("input", {
    id: "lock-text",
    type: "checkbox",
    role: "switch",
    class: "switch",
    checked: lockTextShown,
  });
  lockText.addEventListener("change", () => setLockText(lockText.checked));
  show([
    h("h1", {}, "Settings"),
    h(
      "fieldset",
      {},
      h("legend", {}, "Appearance"),
      h(
        "div",
        { class: "choices" },
        choice("light", "Light"),
        choice("dark", "Dark"),
        choice("system", "Match system"),
      ),
    ),
    h(
      "fieldset",
      {},
      h("legend", {}, "Extra Features"),
      h("label", { class: "check setting", for: "lock-text" }, lockText, h("span", {}, "Lock text")),
      h("p", { class: "hint" }, "Lock away text you type yourself, such as a site password."),
    ),
    links &&
      (links.donate || links.feedbackEmail) &&
      h(
        "fieldset",
        {},
        h("legend", {}, "Support"),
        h(
          "div",
          { class: "links" },
          links.donate && button("Donate", () => openLink("donate"), { icon: heartIcon() }),
          links.feedbackEmail && button("Send feedback", () => openLink("feedback")),
        ),
        // a computer with no mail app can't open the link, so the address shows
        links.feedbackEmail && h("p", { class: "hint address" }, links.feedbackEmail),
      ),
    actions(button("Done", () => void home(), { kind: "primary" })),
  ]);
}

// ----------------------------------------------------------------- setup

function intro(): void {
  show(
    [
      h("p", { class: "step" }, "New code"),
      h("h1", {}, "One key at a time"),
      tapLoop(),
      actions(
        button("Next", passcodeScreen, { kind: "primary", hint: "Enter" }),
        button("Cancel", () => void home(), { kind: "quiet" }),
      ),
    ],
    onKey("Enter", passcodeScreen),
  );
}

function passcodeScreen(): void {
  show(
    [
      beside(
        phone(false),
        h("p", { class: "step" }, "New code"),
        h("h1", {}, "On your iPhone"),
        h(
          "ol",
          { class: "path-to" },
          h("li", {}, "Settings"),
          h("li", {}, "Screen Time"),
          h(
            "li",
            {},
            "Lock Screen Time Settings",
            h("span", { class: "muted" }, " or Change Screen Time Passcode"),
          ),
        ),
        h("p", {}, "Stop at the keypad. Don't tap yet."),
        actions(
          button("Next", coverDots, { kind: "primary", hint: "Enter" }),
          button("Back", intro, { kind: "quiet" }),
        ),
      ),
    ],
    onKey("Enter", coverDots),
  );
}

function coverDots(): void {
  show(
    [
      beside(
        phone(true),
        h("p", { class: "step" }, "New code"),
        h("h1", {}, "Cover the dots"),
        h("p", { class: "muted" }, "So you can't count the digits."),
        actions(
          button("Covered", nameAndDelay, { kind: "primary", hint: "Enter" }),
          button("Back", passcodeScreen, { kind: "quiet" }),
        ),
      ),
    ],
    onKey("Enter", nameAndDelay),
  );
}

/** The unlock-time field, and the line under it that says what it comes to. */
function delayField(): { delay: HTMLInputElement; delayNote: HTMLParagraphElement } {
  const delay = h("input", { id: "delay", value: "1m", autocomplete: "off", spellcheck: "false" });
  const delayNote = h("p", { class: "hint", id: "delay-note" });
  const describe = async () => {
    const text = delay.value;
    const described = await api.describeDelay(text);
    if (delay.value !== text) return;
    delayNote.textContent = described ? `A reveal takes about ${described}.` : "Try 30s, 5m or 2h.";
    delayNote.classList.toggle("error", !described);
  };
  delay.addEventListener("input", () => void describe());
  void describe();
  return { delay, delayNote };
}

function nameAndDelay(): void {
  const label = h("input", { id: "label", value: "iphone", autocomplete: "off", spellcheck: "false", autofocus: true });
  const { delay, delayNote } = delayField();
  const digits = h("input", { id: "digits", type: "number", min: 3, max: 16, value: 4 });
  const taps = h("input", { id: "taps", type: "number", min: 0, max: 200, value: 28 });
  const memoryDigits = h("input", { id: "memory-digits", type: "number", min: 3, max: 9, value: 5 });
  const letters = h("input", { id: "letters", type: "checkbox" });
  const error = h("p", { class: "error", role: "alert" });
  const status = h("p", { class: "muted", role: "status" });
  const fields = h(
    "fieldset",
    {},
    h("label", { for: "label" }, "Name"),
    label,
    h("label", { for: "delay" }, "Unlock time"),
    delay,
    delayNote,
    h(
      "details",
      {},
      h("summary", {}, "Advanced"),
      h(
        "div",
        { class: "fields" },
        h(
          "div",
          {},
          h("label", { for: "digits" }, "Passcode digits"),
          digits,
          h("p", { class: "hint" }, "Screen Time uses 4."),
        ),
        h(
          "div",
          {},
          h("label", { for: "taps" }, "Taps per round"),
          taps,
          h("p", { class: "hint" }, "Roughly. More is harder to follow."),
        ),
        h("div", {}, h("label", { for: "memory-digits" }, "Memory digits"), memoryDigits),
      ),
      h("label", { class: "check", for: "letters" }, letters, h("span", {}, "Letter mode")),
      h("p", { class: "hint" }, "Shows “TAP N” for the 6 key. Codes then use only 2-9."),
    ),
    error,
    actions(
      h("button", { type: "submit", class: "primary" }, "Start"),
      button("Back", coverDots, { kind: "quiet" }),
    ),
  );

  const form = h(
    "form",
    {
      // a refusal is about what the fields held then, not what's typed since
      onInput: () => {
        error.textContent = "";
      },
      onSubmit: async (event: Event) => {
        event.preventDefault();
        if (fields.disabled) return;
        fields.disabled = true;
        error.textContent = "";
        status.textContent = "Sealing the code…";
        try {
          const view = await api.startSetup(
            label.value,
            delay.value,
            Number(digits.value),
            Number(taps.value),
            letters.checked,
            Number(memoryDigits.value),
          );
          showSetup(view);
        } catch (failed) {
          error.textContent = message(failed);
          status.textContent = "";
          fields.disabled = false;
        }
      },
    },
    fields,
  );
  show([h("h1", {}, "New code"), form, status]);
}

let busy = false;
let readyAt = 0;

function showSetup(view: SetupView): void {
  readyAt = performance.now() + INPUT_PAUSE_MS;
  switch (view.kind) {
    case "memory":
      return memoryScreen(view.memoryNumber);
    case "tap":
      return tapScreen(view.round, view.key);
    case "clearField":
      return clearFieldScreen(view.round, view.deletes);
    case "reEnter":
      return reEnterScreen();
    case "finished":
      return finishScreen(view);
    case "stopped":
      return stoppedScreen(view.halfway, view.keptFile);
  }
}

async function act(action: SetupAction): Promise<void> {
  if (busy || performance.now() < readyAt) return;
  busy = true;
  try {
    showSetup(await api.setupAction(action));
  } catch (error) {
    failure(error);
  } finally {
    busy = false;
  }
}

function setupKeys(mistyped: boolean): KeyHandler {
  return (key) => {
    const lower = key.toLowerCase();
    if (key === " " || key === "Enter") void act("next");
    else if (mistyped && lower === "x") void act("mistyped");
    else if (lower === "q") void act("quit");
    else return false;
    return true;
  };
}

function quitButton(): HTMLButtonElement {
  return button("Quit", () => void act("quit"), { kind: "quiet", hint: "Q" });
}

function roundLabel(round: 1 | 2): string {
  return round === 1 ? "Round 1 of 2" : "Round 2 of 2";
}

function memoryScreen(memoryNumber: string): void {
  show(
    [
      h("h1", { class: "center" }, "Remember this number"),
      h("p", { class: "memory-number" }, memoryNumber),
      h("p", { class: "center" }, "Keep repeating it. You'll be asked for it at the end."),
      h("p", { class: "muted center" }, "It won't be shown again."),
      centredActions(button("Begin", () => void act("next"), { kind: "primary", hint: "Space" }), quitButton()),
    ],
    setupKeys(false),
  );
}

function tapScreen(round: 1 | 2, key: string): void {
  const instruction = h(
    "div",
    { class: "instruction" },
    h("span", { class: "tap-word" }, "TAP"),
    h("span", { class: "tap-key" }, key),
  );
  show(
    [
      h("p", { class: "step center" }, roundLabel(round)),
      h("p", { class: "muted center" }, "Keep repeating your number."),
      // every tap has the same layout: the line under the key is always
      // there, and holds a picture only for DELETE
      h(
        "div",
        { class: "instruction-box" },
        instruction,
        h("div", { class: "key-hint" }, key === "DELETE" && backspaceIcon()),
      ),
      centredActions(
        button("Next", () => void act("next"), { kind: "primary", hint: "Space" }),
        button("Mistyped", () => void act("mistyped"), { hint: "X" }),
        quitButton(),
      ),
    ],
    setupKeys(true),
  );
  // the same short animation for every tap, so two DELETEs in a row still
  // look like two instructions
  instruction.animate([{ opacity: 0, transform: "scale(0.96)" }, { opacity: 1, transform: "none" }], {
    duration: 160,
    easing: "ease-out",
  });
}

function clearFieldScreen(round: 1 | 2, deletes: number): void {
  show(
    [
      h("p", { class: "step" }, roundLabel(round)),
      h("h1", { class: "with-icon" }, "Tap DELETE", backspaceIcon(), `${deletes} times`),
      h("p", { class: "muted" }, "That empties the field. Then this round starts over."),
      actions(button("Done", () => void act("next"), { kind: "primary", hint: "Space" }), quitButton()),
    ],
    setupKeys(false),
  );
}

function reEnterScreen(): void {
  show(
    [
      h("h1", {}, "Round 1 done"),
      h("p", {}, "Your iPhone now asks you to re-enter the passcode."),
      h("p", { class: "muted" }, "Round 2 uses different taps for the same code."),
      actions(button("Start round 2", () => void act("next"), { kind: "primary", hint: "Space" }), quitButton()),
    ],
    setupKeys(false),
  );
}

async function closeAndGoHome(notice?: string): Promise<void> {
  try {
    await api.closeSetup();
  } catch (error) {
    failure(error);
    return;
  }
  await home(notice);
}

function stoppedScreen(halfway: boolean, keptFile: boolean): void {
  const phoneStep = halfway
    ? "Tap Cancel on your iPhone. That discards the passcode."
    : "Leave the passcode screen on your iPhone.";
  const file = !keptFile
    ? "Nothing was saved."
    : halfway
      ? "In case the phone took the code anyway, it's kept on Home as unfinished."
      : "The file couldn't be deleted. Discard it on Home.";
  show([
    h("h1", {}, "Stopped"),
    h("p", {}, phoneStep),
    h("p", { class: "muted" }, file),
    actions(button("Home", () => void closeAndGoHome(), { kind: "primary" })),
  ]);
}

/** One box per digit, like a sign-in code. Typing moves along the row, and
 * `onFull` runs as soon as every box holds a digit. */
function digitCells(count: number, labelledBy: string, onFull: (digits: string) => void): HTMLDivElement {
  const cells = Array.from({ length: count }, (_, index) =>
    h("input", {
      class: "cell",
      inputmode: "numeric",
      autocomplete: "off",
      autofocus: index === 0,
      "aria-label": `Digit ${index + 1} of ${count}`,
    }),
  );
  const fill = (start: number, digits: string) => {
    const taken = [...digits].slice(0, count - start);
    taken.forEach((digit, offset) => {
      cells[start + offset].value = digit;
    });
    if (cells.every((cell) => cell.value)) {
      for (const cell of cells) cell.disabled = true;
      onFull(cells.map((cell) => cell.value).join(""));
    } else {
      cells[Math.min(start + taken.length, count - 1)].focus();
    }
  };
  cells.forEach((cell, index) => {
    cell.addEventListener("keydown", (event) => {
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      if (/^\d$/.test(event.key)) {
        event.preventDefault();
        fill(index, event.key);
      } else if (event.key === "Backspace") {
        event.preventDefault();
        if (cell.value) {
          cell.value = "";
        } else if (index > 0) {
          cells[index - 1].value = "";
          cells[index - 1].focus();
        }
      } else if (event.key === "ArrowLeft" && index > 0) {
        event.preventDefault();
        cells[index - 1].focus();
      } else if (event.key === "ArrowRight" && index < count - 1) {
        event.preventDefault();
        cells[index + 1].focus();
      } else if (event.key.length === 1) {
        // any other character has no place in a number
        event.preventDefault();
      }
    });
    cell.addEventListener("paste", (event) => {
      event.preventDefault();
      fill(index, (event.clipboardData?.getData("text") ?? "").replace(/\D/g, ""));
    });
    // text that arrives without a key press, such as a drop
    cell.addEventListener("input", () => {
      const digits = cell.value.replace(/\D/g, "");
      cell.value = "";
      fill(index, digits);
    });
    cell.addEventListener("focus", () => cell.select());
  });
  return h("div", { class: "cells", role: "group", "aria-labelledby": labelledBy }, ...cells);
}

function finishScreen(view: Finished): void {
  let asked = false;
  const check = async (guess: string) => {
    if (asked) return;
    asked = true;
    try {
      const result = await api.checkMemory(guess);
      memoryResult(view, result.correct, result.memoryNumber);
    } catch (error) {
      failure(error);
    }
  };
  show([
    h("h1", { id: "guess-title", class: "center" }, "What was your number?"),
    digitCells(view.memoryDigits, "guess-title", (digits) => void check(digits)),
    centredActions(button("Forgot", () => void check(""), { kind: "quiet" })),
  ]);
}

function memoryResult(view: Finished, correct: boolean, memoryNumber: string): void {
  const next = () => appleId(view);
  // the last digit confirmed by itself, so an Enter pressed out of habit
  // right after it mustn't skip this screen
  const keysFrom = performance.now() + RESULT_PAUSE_MS;
  show(
    [
      h("h1", { class: "center" }, correct ? "Correct" : `It was ${memoryNumber}`),
      h("p", { class: "muted center" }, correct ? "It did its job." : "No matter. It did its job."),
      centredActions(button("Continue", next, { kind: "primary", hint: "Enter" })),
    ],
    onKey("Enter", () => {
      if (performance.now() >= keysFrom) next();
    }),
  );
}

/** A file that has just been saved, as its closing screens need it. */
interface Saved {
  path: string;
  /** How long a reveal takes, in words. */
  delay: string;
  /** The synced folder, if there is one to offer. */
  backup: string | null;
  /** Copies the file into that folder and says where the copy went. */
  backUp: () => Promise<string>;
  /** What the file holds: "code" or "text". */
  holds: string;
  /** A last line for the closing screen. */
  advice?: string;
  leave: () => void;
}

function appleId(view: Finished): void {
  const saved: Saved = {
    path: view.path,
    delay: view.delay,
    backup: view.backup,
    backUp: api.backUpFinished,
    holds: "code",
    leave: () => void closeAndGoHome(),
  };
  const next = () =>
    view.pending
      ? summary(saved, { note: "Saved as an unfinished setup, because its file name was taken. Choose Keep on Home." })
      : backupOffer(saved);
  show([
    h("p", { class: "step" }, "The passcode is set"),
    h("h1", {}, "Enter your Apple ID on the phone"),
    h("p", {}, "Don't skip it. It's the fallback if the recovery file is lost."),
    actions(button("Done", next, { kind: "primary" })),
  ]);
}

function backupOffer(saved: Saved): void {
  if (!saved.backup) {
    summary(saved, { note: "Keep a copy off this computer. To be offered a folder, set LOCKER_BACKUP_DIR." });
    return;
  }
  show([
    h("h1", {}, "Back it up?"),
    h("p", { class: "row-path" }, saved.backup),
    h("p", { class: "muted" }, "The copy is time-locked too."),
    actions(
      button(
        "Back up",
        async () => {
          try {
            summary(saved, { backup: await saved.backUp() });
          } catch (error) {
            summary(saved, { note: message(error) });
          }
        },
        { kind: "primary" },
      ),
      button("Skip", () => summary(saved), { kind: "quiet" }),
    ),
  ]);
}

function summary(saved: Saved, extra: { backup?: string; note?: string } = {}): void {
  show([
    h("h1", {}, "All done"),
    h(
      "dl",
      { class: "facts" },
      h("dt", {}, "File"),
      h("dd", { class: "row-path" }, saved.path),
      extra.backup && h("dt", {}, "Backup"),
      extra.backup && h("dd", { class: "row-path" }, extra.backup),
    ),
    extra.note && h("p", {}, extra.note),
    h("p", {}, `To get the ${saved.holds}, choose Reveal on Home. It takes about ${saved.delay}.`),
    saved.advice && h("p", { class: "muted" }, saved.advice),
    actions(button("Done", saved.leave, { kind: "primary" })),
  ]);
}

// -------------------------------------------------------------- own text

/** The most a file can hold, in UTF-8 bytes. The Rust side has the last word. */
const MAX_TEXT_BYTES = 32;

/** Text the person brings themselves: no phone and no taps, so one form. The
 * text stays in view while it's typed, because a typo locked away can't be
 * found until the reveal. */
function lockTextScreen(): void {
  const label = h("input", { id: "label", required: true, autocomplete: "off", spellcheck: "false", autofocus: true });
  const text = h("input", {
    id: "text",
    class: "typed",
    required: true,
    autocomplete: "off",
    autocapitalize: "off",
    spellcheck: "false",
  });
  const textNote = h("p", { class: "hint", id: "text-note" });
  const { delay, delayNote } = delayField();
  const error = h("p", { class: "error", role: "alert" });
  const status = h("p", { class: "muted", role: "status" });

  const problem = (): string => {
    const typed = text.value;
    if (typed !== typed.trim()) return "It starts or ends with a space.";
    const over = new TextEncoder().encode(typed).length - MAX_TEXT_BYTES;
    if (over <= 0) return "";
    // accents and emoji take more than one of the 32
    return /^[\x20-\x7e]*$/.test(typed) ? `Too long by ${over}.` : "Too long. Accents and emoji count for more.";
  };
  const check = () => {
    const found = problem();
    textNote.textContent = found || `Up to ${MAX_TEXT_BYTES} characters.`;
    textNote.classList.toggle("error", found !== "");
  };
  text.addEventListener("input", check);
  check();

  const fields = h(
    "fieldset",
    { class: "own-text" },
    h("label", { for: "label" }, "Name"),
    label,
    h("label", { for: "text" }, "Text"),
    text,
    textNote,
    h("label", { for: "delay" }, "Unlock time"),
    delay,
    delayNote,
    error,
    actions(
      h("button", { type: "submit", class: "primary" }, "Lock"),
      button("Cancel", () => void home(), { kind: "quiet" }),
    ),
  );
  const form = h(
    "form",
    {
      // a refusal is about what the fields held then, not what's typed since
      onInput: () => {
        error.textContent = "";
      },
      onSubmit: async (event: Event) => {
        event.preventDefault();
        if (fields.disabled || problem()) return;
        fields.disabled = true;
        error.textContent = "";
        status.textContent = "Locking…";
        try {
          const locked = await api.lockText(label.value, text.value, delay.value);
          backupOffer({
            ...locked,
            backUp: () => api.backUpSaved(locked.path),
            holds: "text",
            advice: "Now clear any other copy of it, such as your clipboard.",
            leave: () => void home(),
          });
        } catch (failed) {
          error.textContent = message(failed);
          status.textContent = "";
          fields.disabled = false;
        }
      },
    },
    fields,
  );
  show([h("h1", {}, "Lock text"), form, status]);
}

// ---------------------------------------------------------------- reveal

async function revealScreen(path: string): Promise<void> {
  show([h("h1", {}, "Reveal"), h("p", { class: "muted" }, "Checking…")]);
  let info: RevealInfo;
  try {
    info = await api.revealInfo(path);
  } catch (error) {
    failure(error);
    return;
  }
  show([
    h("h1", {}, `Reveal “${info.label}”?`),
    h(
      "dl",
      { class: "facts" },
      h("dt", {}, "Created"),
      h("dd", {}, info.created),
      h("dt", {}, "File"),
      h("dd", { class: "row-path" }, info.path),
      h("dt", {}, "Takes"),
      h("dd", {}, info.legacy ? "no time (legacy file)" : `about ${info.estimate ?? "a moment"}`),
    ),
    h("p", { class: "callout" }, "Once you see the code, the lock is pointless."),
    // the wait is what guards a code, so this takes a button, not a typed word
    actions(
      button("Reveal", () => (info.legacy ? revealLegacy(info) : progressScreen(info)), { kind: "accent" }),
      button("Cancel", () => void home(), { kind: "quiet" }),
    ),
  ]);
}

function onRevealEvent(info: RevealInfo, event: RevealEvent): void {
  switch (event.kind) {
    case "revealed":
      return codeScreen(info, event.code);
    case "damaged":
      return show([
        h("h1", {}, "The file is damaged"),
        actions(button("Home", () => void home(), { kind: "primary" })),
      ]);
    case "cancelled":
      return void home();
    case "progress":
      return;
  }
}

/** A legacy file stores its code in plain text, so it answers at once. */
function revealLegacy(info: RevealInfo): void {
  api.reveal(info.path, (event) => onRevealEvent(info, event)).catch(failure);
}

function progressScreen(info: RevealInfo): void {
  const fill = h("div", { class: "bar-fill" });
  const status = h("p", { class: "muted", role: "status" }, "Starting…");
  const started = performance.now();
  let cancelling = false;
  const cancel = () => {
    cancelling = true;
    status.textContent = "Cancelling…";
    void api.cancelReveal();
  };
  show(
    [
      h("h1", {}, "Unlocking"),
      h("div", { class: "bar" }, fill),
      status,
      h("p", { class: "muted" }, "Cancelling loses all progress."),
      actions(button("Cancel", cancel, { kind: "quiet", hint: "Esc" })),
    ],
    onKey("Escape", cancel),
  );
  api
    .reveal(info.path, (event) => {
      if (event.kind !== "progress") {
        onRevealEvent(info, event);
        return;
      }
      const fraction = event.done / event.total;
      fill.style.width = `${(fraction * 100).toFixed(1)}%`;
      if (cancelling) return;
      const elapsed = (performance.now() - started) / 1000;
      const remaining = (elapsed / event.done) * (event.total - event.done);
      status.textContent = `${(fraction * 100).toFixed(1)}%, about ${fmtDuration(remaining)} left`;
    })
    .catch(failure);
}

/** Up to 16 digits is a passcode. Anything else is text someone locked
 * themselves, which is drawn so that l, 1, O and 0 can be told apart. */
function isPasscode(code: string): boolean {
  return /^\d{1,16}$/.test(code);
}

function codeScreen(info: RevealInfo, code: string): void {
  const size = !isPasscode(code) ? " text" : code.length > 12 ? " long" : "";
  show([
    h("p", { class: "step center" }, info.label),
    h("h1", { class: `revealed-code${size}` }, code),
    centredActions(
      button("Done", () => void home(), { kind: "primary" }),
      button("Delete the file", () => void deleteScreen(info.path), { kind: "delete" }),
    ),
  ]);
}

// ---------------------------------------------------------------- delete

async function deleteScreen(path: string): Promise<void> {
  let info: DeleteInfo;
  try {
    info = await api.deleteInfo(path);
  } catch (error) {
    failure(error);
    return;
  }
  // deleting can be undone for 30 days, so it takes a button, not a typed word
  let asked = false;
  const remove = async () => {
    if (asked) return;
    asked = true;
    try {
      const outcomes = await api.deleteSaved(path);
      if (allWent(outcomes)) await home("Deleted.");
      else leftBehind("Not everything was deleted", outcomes);
    } catch (error) {
      failure(error);
    }
  };
  show([
    h("p", { class: "step" }, `Created ${info.created}`),
    h("h1", {}, `Delete “${info.label}”?`),
    h("ul", { class: "paths" }, ...info.copies.map((copy) => h("li", {}, copy))),
    h("p", { class: "callout" }, "It stays in Recently deleted for 30 days."),
    actions(
      button("Delete", () => void remove(), { kind: "danger" }),
      button("Cancel", () => void home(), { kind: "quiet" }),
    ),
  ]);
}

function allWent(outcomes: DeleteOutcome[]): boolean {
  return outcomes.every((outcome) => !outcome.error);
}

/** Names the copies of a file that couldn't be moved or removed. */
function leftBehind(title: string, outcomes: DeleteOutcome[]): void {
  show([
    h("h1", {}, title),
    h(
      "ul",
      { class: "paths" },
      ...outcomes
        .filter((outcome) => outcome.error)
        .map((outcome) => h("li", {}, `${outcome.path} (${outcome.error})`)),
    ),
    actions(button("Home", () => void home(), { kind: "primary" })),
  ]);
}

// ------------------------------------------------------ recently deleted

async function binScreen(notice?: string): Promise<void> {
  let deleted: DeletedCode[];
  try {
    deleted = await api.deletedCodes();
  } catch (error) {
    failure(error);
    return;
  }
  if (deleted.length === 0) {
    await home(notice);
    return;
  }
  let asked = false;
  const restore = async (item: DeletedCode) => {
    if (asked) return;
    asked = true;
    try {
      const outcomes = await api.restoreDeleted(item.id);
      if (allWent(outcomes)) await home("Restored.");
      else leftBehind("Not everything was restored", outcomes);
    } catch (error) {
      failure(error);
    }
  };
  show(
    [
      h(
        "div",
        { class: "head" },
        h("h1", {}, "Recently deleted"),
        button("Back", () => void home(), { kind: "quiet" }),
      ),
      notice && h("p", { class: "notice", role: "status" }, notice),
      h(
        "ul",
        { class: "codes" },
        ...deleted.map((item) =>
          h(
            "li",
            { class: "row" },
            h(
              "div",
              { class: "row-main" },
              h("div", { class: "row-title" }, item.label),
              // there only if the code was revealed before it was deleted
              item.code && h("div", { class: isPasscode(item.code) ? "row-code" : "row-code text" }, item.code),
              h(
                "div",
                { class: "row-meta" },
                `${item.created} · ${item.daysLeft} day${item.daysLeft === 1 ? "" : "s"} left`,
              ),
            ),
            h(
              "div",
              { class: "row-actions" },
              button("Restore", () => void restore(item)),
              button("Delete forever", () => deleteForever(item), { kind: "delete" }),
            ),
          ),
        ),
      ),
    ],
    null,
    "list",
  );
}

function deleteForever(item: DeletedCode): void {
  show([
    h("p", { class: "step" }, `Created ${item.created}`),
    h("h1", {}, `Delete “${item.label}” forever?`),
    h("ul", { class: "paths" }, ...item.copies.map((copy) => h("li", {}, copy))),
    h("p", { class: "callout danger" }, "If this code is still set anywhere, it's gone for good."),
    typedConfirmation(
      "delete",
      "Delete forever",
      async (typed) => {
        try {
          const outcomes = await api.deleteForever(item.id, typed);
          if (allWent(outcomes)) await binScreen("Deleted.");
          else leftBehind("Not everything was deleted", outcomes);
        } catch (error) {
          failure(error);
        }
      },
      () => void binScreen(),
    ),
  ]);
}

// ----------------------------------------------------------------- start

async function start(): Promise<void> {
  try {
    // a reload during a setup carries on where it was
    const view = await api.setupView();
    if (view) showSetup(view);
    else await home();
  } catch (error) {
    failure(error);
  }
}

void start();
