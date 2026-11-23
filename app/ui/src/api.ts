// Calls into the Rust side, which does all file access. The page itself has
// no file system or network access.

interface TauriChannel<T> {
  onmessage: (message: T) => void;
}

interface TauriGlobal {
  core: {
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
    Channel: new <T>() => TauriChannel<T>;
  };
}

declare global {
  interface Window {
    __TAURI__: TauriGlobal;
  }
}

export interface Overview {
  folder: string;
  backup: string | null;
  /** Newest first. */
  codes: SavedCode[];
  unfinished: Unfinished[];
  /** How many codes wait in Recently deleted. */
  deleted: number;
}

export interface SavedCode {
  path: string;
  label: string;
  created: string;
  cost: string;
  readable: boolean;
  backupOnly: boolean;
}

export interface Unfinished {
  path: string;
  label: string;
  created: string;
}

/** One setup screen. A tap carries only its round and key. */
export type SetupView =
  | { kind: "memory"; memoryNumber: string }
  /** `key` is a digit, a letter (letter mode) or DELETE. */
  | { kind: "tap"; round: 1 | 2; key: string }
  | { kind: "clearField"; round: 1 | 2; deletes: number }
  | { kind: "reEnter" }
  | {
      kind: "finished";
      path: string;
      pending: boolean;
      label: string;
      delay: string;
      backup: string | null;
      memoryDigits: number;
    }
  | { kind: "stopped"; halfway: boolean; keptFile: boolean };

export type SetupAction = "next" | "mistyped" | "quit";

export interface MemoryCheck {
  correct: boolean;
  memoryNumber: string;
}

/** Text the person typed themselves, now locked in the file at `path`. */
export interface LockedText {
  path: string;
  delay: string;
  backup: string | null;
}

export interface RevealInfo {
  path: string;
  label: string;
  created: string;
  legacy: boolean;
  estimate: string | null;
}

export type RevealEvent =
  | { kind: "progress"; done: number; total: number }
  | { kind: "revealed"; code: string }
  | { kind: "damaged" }
  | { kind: "cancelled" };

export interface DeleteInfo {
  label: string;
  created: string;
  copies: string[];
}

export interface DeleteOutcome {
  path: string;
  error: string | null;
}

/** A code in Recently deleted. `code` is there only if it had been revealed
 * before it was deleted. */
export interface DeletedCode {
  id: string;
  label: string;
  created: string;
  daysLeft: number;
  code: string | null;
  copies: string[];
}

/** The app's own links. Settings offers only the ones that are set up. */
export interface Links {
  donate: boolean;
  /** Shown as well as opened, for a computer with no mail app. */
  feedbackEmail: string | null;
}

export type Link = "donate" | "feedback";

function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return window.__TAURI__.core.invoke<T>(command, args);
}

export const api = {
  overview: () => call<Overview>("overview"),
  keepUnfinished: (path: string) => call<string>("keep_unfinished", { path }),
  discardUnfinished: (path: string, confirmation: string) =>
    call<void>("discard_unfinished", { path, confirmation }),
  describeDelay: (text: string) => call<string | null>("describe_delay", { text }),
  startSetup: (
    label: string,
    delay: string,
    digits: number,
    taps: number,
    letters: boolean,
    memoryDigits: number,
  ) => call<SetupView>("start_setup", { label, delay, digits, taps, letters, memoryDigits }),
  setupView: () => call<SetupView | null>("setup_view"),
  setupAction: (action: SetupAction) => call<SetupView>("setup_action", { action }),
  checkMemory: (guess: string) => call<MemoryCheck>("check_memory", { guess }),
  backUpFinished: () => call<string>("back_up_finished"),
  closeSetup: () => call<void>("close_setup"),
  lockText: (label: string, text: string, delay: string) => call<LockedText>("lock_text", { label, text, delay }),
  backUpSaved: (path: string) => call<string>("back_up_saved", { path }),
  revealInfo: (path: string) => call<RevealInfo>("reveal_info", { path }),
  reveal: (path: string, onEvent: (event: RevealEvent) => void) => {
    const channel = new window.__TAURI__.core.Channel<RevealEvent>();
    channel.onmessage = onEvent;
    return call<void>("reveal", { path, onEvent: channel });
  },
  cancelReveal: () => call<void>("cancel_reveal"),
  deleteInfo: (path: string) => call<DeleteInfo>("delete_info", { path }),
  deleteSaved: (path: string) => call<DeleteOutcome[]>("delete_saved", { path }),
  deletedCodes: () => call<DeletedCode[]>("deleted_codes"),
  restoreDeleted: (id: string) => call<DeleteOutcome[]>("restore_deleted", { id }),
  deleteForever: (id: string, confirmation: string) =>
    call<DeleteOutcome[]>("delete_forever", { id, confirmation }),
  links: () => call<Links>("links"),
  openLink: (link: Link) => call<void>("open_link", { link }),
};
