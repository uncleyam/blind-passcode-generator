//! The desktop app, Blind Passcode Generator: a thin shell over `locker-core`. The
//! code keeps the project's first name, Locker.
//!
//! During a setup the UI only ever receives the next instruction, and it sees
//! a code only when a reveal finishes. Every file access happens here, and
//! paths coming back from the UI must name a file Locker itself listed.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use locker_core::recovery::{self, fmt_duration, parse_delay};
use locker_core::sequence::{MAX_DIGITS, MIN_DIGITS};
use locker_core::setup::{
    Action, Key, MAX_MEMORY_DIGITS, MIN_MEMORY_DIGITS, Options, Round, Screen, Setup,
};
use locker_core::storage::{Deleted, Store, local_now};
use locker_core::text::{self, MAX_TEXT_BYTES, TextError};
use locker_core::timelock::{self, SolveError};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

/// Errors reach the UI as text it shows as it is.
type Reply<T> = Result<T, String>;

/// More taps than this per round helps nobody, and the old command line had
/// no limit only because nobody types that much.
const MAX_TAPS: i64 = 200;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

struct Locker {
    store: Option<Store>,
    setup: Mutex<Option<ActiveSetup>>,
    starting: AtomicBool,
    revealing: Arc<AtomicBool>,
    cancel_reveal: Arc<AtomicBool>,
    /// The code a reveal last showed, and the file it came from. Deleting
    /// that file while its code is still up puts the code in the bin with it.
    /// Home forgets it.
    revealed: Arc<Mutex<Option<(PathBuf, String)>>>,
}

struct ActiveSetup {
    setup: Setup,
    label: String,
    delay: f64,
    digits: usize,
}

impl Locker {
    fn store(&self) -> Reply<&Store> {
        self.store
            .as_ref()
            .ok_or_else(|| "Blind Passcode Generator couldn't find your home folder.".to_string())
    }

    fn active(&self) -> MutexGuard<'_, Option<ActiveSetup>> {
        lock(&self.setup)
    }

    /// A path from the UI, accepted only if it names a saved code right now.
    fn saved(&self, path: &str) -> Reply<PathBuf> {
        let path = PathBuf::from(path);
        if self.store()?.all_recoveries().contains(&path) {
            Ok(path)
        } else {
            Err("That saved code isn't there any more.".into())
        }
    }

    /// A path from the UI, accepted only if it names an unfinished setup.
    fn unfinished(&self, path: &str) -> Reply<PathBuf> {
        let path = PathBuf::from(path);
        if self.store()?.pending_files().contains(&path) {
            Ok(path)
        } else {
            Err("That unfinished setup isn't there any more.".into())
        }
    }

    /// A name from the UI, accepted only if the bin holds it right now.
    fn binned(&self, id: &str) -> Reply<Deleted> {
        self.store()?
            .deleted()
            .into_iter()
            .find(|deleted| deleted.name == id)
            .ok_or_else(|| "That isn't in Recently deleted any more.".to_string())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

/// The typed confirmations compare the way the old command line did.
fn confirmed(typed: &str, word: &str) -> bool {
    typed.trim().to_lowercase() == word
}

// ------------------------------------------------------------------ home

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Overview {
    folder: String,
    backup: Option<String>,
    /// Newest first.
    codes: Vec<SavedCode>,
    unfinished: Vec<Unfinished>,
    /// How many codes wait in Recently deleted.
    deleted: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedCode {
    path: String,
    label: String,
    created: String,
    cost: String,
    readable: bool,
    /// Only a synced backup is left; the original is gone.
    backup_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Unfinished {
    path: String,
    label: String,
    created: String,
}

/// The label, creation time and reveal cost, as `locker list` shows them.
fn describe(path: &Path) -> (String, String, String, bool) {
    let Some(data) = recovery::load(path) else {
        return ("(unreadable)".into(), "unknown".into(), "?".into(), false);
    };
    let label = data.label().unwrap_or_else(|| "(no label)".into());
    let cost = if data.pin().is_some() {
        "instant (legacy file)".to_string()
    } else if let Some(delay) = data.delay_seconds() {
        format!("~{} to reveal", fmt_duration(delay))
    } else {
        "time-locked".to_string()
    };
    (label, data.created(), cost, true)
}

#[tauri::command]
fn overview(state: State<'_, Locker>) -> Reply<Overview> {
    let store = state.store()?;
    // Home is up, so no revealed code is on the screen any more
    *lock(&state.revealed) = None;
    store.purge_expired(local_now());
    let codes = store
        .all_recoveries()
        .iter()
        .rev()
        .map(|path| {
            let (label, created, cost, readable) = describe(path);
            SavedCode {
                path: shown(path),
                label,
                created,
                cost,
                readable,
                backup_only: store.backup.as_deref() == path.parent(),
            }
        })
        .collect();
    let unfinished = store
        .pending_files()
        .iter()
        .map(|path| {
            let (label, created, _, _) = describe(path);
            Unfinished {
                path: shown(path),
                label,
                created,
            }
        })
        .collect();
    Ok(Overview {
        folder: shown(&store.main),
        backup: store.backup.as_deref().map(shown),
        codes,
        unfinished,
        deleted: store.deleted().len(),
    })
}

#[tauri::command]
fn keep_unfinished(state: State<'_, Locker>, path: String) -> Reply<String> {
    let path = state.unfinished(&path)?;
    let saved = state
        .store()?
        .finish_pending(&path)
        .map_err(|error| format!("Could not save it ({error})."))?;
    Ok(shown(&saved))
}

#[tauri::command]
fn discard_unfinished(state: State<'_, Locker>, path: String, confirmation: String) -> Reply<()> {
    if !confirmed(&confirmation, "delete") {
        return Err("Type delete to confirm.".into());
    }
    let path = state.unfinished(&path)?;
    std::fs::remove_file(&path).map_err(|error| format!("Could not delete it ({error})."))
}

// ----------------------------------------------------------------- setup

/// One setup screen, as the UI sees it. A tap carries only its round and key.
#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum SetupView {
    Memory {
        memory_number: String,
    },
    Tap {
        round: u8,
        key: String,
    },
    /// `deletes` taps of DELETE empty the field even with the dots covered.
    ClearField {
        round: u8,
        deletes: usize,
    },
    ReEnter,
    Finished {
        path: String,
        pending: bool,
        label: String,
        delay: String,
        backup: Option<String>,
        /// How many digits the memory check should ask for.
        memory_digits: usize,
    },
    Stopped {
        halfway: bool,
        kept_file: bool,
    },
}

fn round_number(round: Round) -> u8 {
    match round {
        Round::Set => 1,
        Round::Confirm => 2,
    }
}

fn view(active: &ActiveSetup, store: &Store) -> SetupView {
    match active.setup.screen() {
        // the only time the memory number goes out; it's never shown again
        Screen::Memory => SetupView::Memory {
            memory_number: active.setup.memory_number().to_string(),
        },
        Screen::Tap { round, key } => SetupView::Tap {
            round: round_number(round),
            key: match key {
                Key::Digit(digit) => digit.to_string(),
                Key::Letter(letter) => letter.to_string(),
                Key::Delete => "DELETE".to_string(),
            },
        },
        Screen::ClearField { round } => SetupView::ClearField {
            round: round_number(round),
            // the field never holds more than one digit fewer than the code
            deletes: active.digits - 1,
        },
        Screen::ReEnter => SetupView::ReEnter,
        Screen::Finished { path, pending } => SetupView::Finished {
            path: shown(&path),
            pending,
            label: active.label.clone(),
            delay: fmt_duration(active.delay),
            backup: (!pending)
                .then(|| store.backup.as_deref().map(shown))
                .flatten(),
            memory_digits: active.setup.memory_number().len(),
        },
        Screen::Stopped { halfway, kept_file } => SetupView::Stopped { halfway, kept_file },
    }
}

/// How long a reveal of this delay would take, or None if it doesn't parse.
#[tauri::command]
fn describe_delay(text: String) -> Option<String> {
    parse_delay(&text).map(fmt_duration)
}

/// Clears the flag that lets only one setup start at a time.
struct Starting<'a>(&'a AtomicBool);

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
async fn start_setup(
    state: State<'_, Locker>,
    label: String,
    delay: String,
    digits: usize,
    taps: i64,
    letters: bool,
    memory_digits: usize,
) -> Reply<SetupView> {
    let store = state.store()?.clone();
    if state.starting.swap(true, Ordering::SeqCst) {
        return Err("A setup is already starting.".into());
    }
    let _starting = Starting(&state.starting);
    if state
        .active()
        .as_ref()
        .is_some_and(|active| !active.setup.is_over())
    {
        return Err("A setup is already running.".into());
    }
    let delay = parse_delay(&delay).ok_or("Could not read the unlock time. Try 30s, 5m or 2h.")?;
    if !(MIN_DIGITS..=MAX_DIGITS).contains(&digits) {
        return Err(format!(
            "The code must be {MIN_DIGITS}-{MAX_DIGITS} digits."
        ));
    }
    if !(0..=MAX_TAPS).contains(&taps) {
        return Err(format!("Taps per round must be 0-{MAX_TAPS}."));
    }
    if !(MIN_MEMORY_DIGITS..=MAX_MEMORY_DIGITS).contains(&memory_digits) {
        return Err(format!(
            "The memory number must be {MIN_MEMORY_DIGITS}-{MAX_MEMORY_DIGITS} digits."
        ));
    }
    let label = match label.trim() {
        "" => "iphone".to_string(),
        label => label.to_string(),
    };
    let options = Options {
        digits,
        target_taps: taps,
        label: label.clone(),
        delay,
        letters,
        memory_digits,
    };
    let begin_store = store.clone();
    let setup = tauri::async_runtime::spawn_blocking(move || Setup::begin(begin_store, &options))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("Could not start: {error}."))?;
    let active = ActiveSetup {
        setup,
        label,
        delay,
        digits,
    };
    let shown_view = view(&active, &store);
    *state.active() = Some(active);
    Ok(shown_view)
}

/// The current setup screen, if a setup is running or just ended. The UI asks
/// for this when it starts, so a reload carries on where it was.
#[tauri::command]
fn setup_view(state: State<'_, Locker>) -> Reply<Option<SetupView>> {
    let store = state.store()?;
    Ok(state.active().as_ref().map(|active| view(active, store)))
}

#[tauri::command]
fn setup_action(state: State<'_, Locker>, action: String) -> Reply<SetupView> {
    let action = match action.as_str() {
        "next" => Action::Next,
        "mistyped" => Action::Mistyped,
        "quit" => Action::Quit,
        other => return Err(format!("Unknown action {other}.")),
    };
    let store = state.store()?;
    let mut active = state.active();
    let active = active.as_mut().ok_or("No setup is running.")?;
    active
        .setup
        .advance(action)
        .map_err(|error| format!("The setup hit a problem: {error}."))?;
    Ok(view(active, store))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryCheck {
    correct: bool,
    memory_number: String,
}

/// Checks the memory number once the code is on the phone, and gives it back.
#[tauri::command]
fn check_memory(state: State<'_, Locker>, guess: String) -> Reply<MemoryCheck> {
    let active = state.active();
    let active = active.as_ref().ok_or("No setup is running.")?;
    if !matches!(active.setup.screen(), Screen::Finished { .. }) {
        return Err("The memory check comes after the taps.".into());
    }
    Ok(MemoryCheck {
        correct: active.setup.memory_matches(&guess),
        memory_number: active.setup.memory_number().to_string(),
    })
}

#[tauri::command]
fn back_up_finished(state: State<'_, Locker>) -> Reply<String> {
    let store = state.store()?;
    let active = state.active();
    let active = active.as_ref().ok_or("No setup is running.")?;
    let Screen::Finished {
        path,
        pending: false,
    } = active.setup.screen()
    else {
        return Err("There is no finished code to back up.".into());
    };
    backed_up(store, &path)
}

/// Copies a saved file into the synced folder and says where the copy went.
fn backed_up(store: &Store, path: &Path) -> Reply<String> {
    match store.copy_to_backup(path) {
        Ok(Some(copy)) => Ok(shown(&copy)),
        Ok(None) => Err("No synced folder was found.".into()),
        Err(error) => Err(format!(
            "Backup failed ({error}). The main file is still fine."
        )),
    }
}

/// Forgets a setup that has finished or stopped.
#[tauri::command]
fn close_setup(state: State<'_, Locker>) -> Reply<()> {
    let mut active = state.active();
    if active
        .as_ref()
        .is_some_and(|active| !active.setup.is_over())
    {
        return Err("The setup is still running.".into());
    }
    *active = None;
    Ok(())
}

// -------------------------------------------------------------- own text

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LockedText {
    path: String,
    delay: String,
    backup: Option<String>,
}

fn text_problem(error: TextError) -> String {
    match error {
        TextError::Empty => "Type the text to lock.".into(),
        TextError::TooLong => format!(
            "That's too long. The most is {MAX_TEXT_BYTES} characters, and accents and emoji count for more."
        ),
        TextError::SpaceAtAnEnd => "The text starts or ends with a space.".into(),
        TextError::ControlCharacter => "The text has a line break or a tab in it.".into(),
        TextError::InvalidDelay => "The unlock time must be more than zero.".into(),
        TextError::Io(error) => format!("Could not save it ({error})."),
    }
}

/// Locks away text the person typed themselves, exactly as it is. Nothing
/// goes on a phone, so there are no taps and the file is saved at once.
#[tauri::command]
async fn lock_text(
    state: State<'_, Locker>,
    label: String,
    text: String,
    delay: String,
) -> Reply<LockedText> {
    let store = state.store()?.clone();
    if state.starting.swap(true, Ordering::SeqCst) {
        return Err("It's already being locked.".into());
    }
    let _starting = Starting(&state.starting);
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err("Give it a name.".into());
    }
    let delay = parse_delay(&delay).ok_or("Could not read the unlock time. Try 30s, 5m or 2h.")?;
    let saving = store.clone();
    let path =
        tauri::async_runtime::spawn_blocking(move || text::lock(&saving, &text, &label, delay))
            .await
            .map_err(|error| error.to_string())?
            .map_err(text_problem)?;
    Ok(LockedText {
        path: shown(&path),
        delay: fmt_duration(delay),
        backup: store.backup.as_deref().map(shown),
    })
}

/// Backs up a file that no setup made, such as locked text.
#[tauri::command]
fn back_up_saved(state: State<'_, Locker>, path: String) -> Reply<String> {
    let path = state.saved(&path)?;
    backed_up(state.store()?, &path)
}

// ---------------------------------------------------------------- reveal

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevealInfo {
    path: String,
    label: String,
    created: String,
    legacy: bool,
    /// How long a reveal takes on this computer, measured now.
    estimate: Option<String>,
}

#[tauri::command]
async fn reveal_info(state: State<'_, Locker>, path: String) -> Reply<RevealInfo> {
    let path = state.saved(&path)?;
    let data = recovery::load(&path)
        .ok_or_else(|| format!("Could not decode {}. Is it a recovery file?", shown(&path)))?;
    let (label, created) = (
        data.label().unwrap_or_else(|| "(none)".into()),
        data.created(),
    );
    if data.pin().is_some() {
        return Ok(RevealInfo {
            path: shown(&path),
            label,
            created,
            legacy: true,
            estimate: None,
        });
    }
    let damaged = "Integrity check failed: the file looks damaged.";
    let puzzle = data.puzzle().ok_or(damaged)?;
    // the stored delay reflects the program that sealed the file, so measure
    let seconds = tauri::async_runtime::spawn_blocking(move || puzzle.estimated_seconds())
        .await
        .map_err(|error| error.to_string())?
        .ok_or(damaged)?;
    Ok(RevealInfo {
        path: shown(&path),
        label,
        created,
        legacy: false,
        estimate: Some(fmt_duration(seconds)),
    })
}

#[derive(Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum RevealEvent {
    Progress { done: u64, total: u64 },
    Revealed { code: String },
    Damaged,
    Cancelled,
}

/// Keeps the computer from sleeping while a reveal runs. The screen may still
/// turn off. Failing to get this only means a long reveal could pause.
fn keep_awake() -> Option<keepawake::KeepAwake> {
    keepawake::Builder::default()
        .idle(true)
        .reason("Revealing a code")
        .app_name("Blind Passcode Generator")
        .app_reverse_domain("com.uncleyam.blindpasscodegenerator")
        .create()
        .ok()
}

/// Reveals a saved code. A legacy file answers at once. A time-locked one
/// grinds on a worker thread, reporting progress until it finishes or is
/// cancelled; progress isn't saved. The wait is what guards a code, so no
/// word has to be typed first.
#[tauri::command]
fn reveal(state: State<'_, Locker>, path: String, on_event: Channel<RevealEvent>) -> Reply<()> {
    let path = state.saved(&path)?;
    let data = recovery::load(&path).ok_or("Could not decode that file.")?;
    if let Some(pin) = data.pin() {
        *lock(&state.revealed) = Some((path, pin.clone()));
        return on_event
            .send(RevealEvent::Revealed { code: pin })
            .map_err(|error| error.to_string());
    }
    let puzzle = data
        .puzzle()
        .ok_or("Integrity check failed: the file looks damaged.")?;
    if state.revealing.swap(true, Ordering::SeqCst) {
        return Err("A reveal is already running.".into());
    }
    state.cancel_reveal.store(false, Ordering::SeqCst);
    let (revealing, cancel) = (state.revealing.clone(), state.cancel_reveal.clone());
    let revealed = state.revealed.clone();
    std::thread::spawn(move || {
        let _awake = keep_awake();
        let mut last_report: Option<Instant> = None;
        let result = timelock::solve(&puzzle, |done, total| {
            if cancel.load(Ordering::SeqCst) {
                return ControlFlow::Break(());
            }
            if last_report.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                last_report = Some(Instant::now());
                let _ = on_event.send(RevealEvent::Progress { done, total });
            }
            ControlFlow::Continue(())
        });
        let _ = on_event.send(match result {
            Ok(code) => {
                *lock(&revealed) = Some((path, code.clone()));
                RevealEvent::Revealed { code }
            }
            Err(SolveError::Damaged) => RevealEvent::Damaged,
            Err(SolveError::Cancelled) => RevealEvent::Cancelled,
        });
        revealing.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
fn cancel_reveal(state: State<'_, Locker>) {
    state.cancel_reveal.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------- delete

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteInfo {
    label: String,
    created: String,
    copies: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteOutcome {
    path: String,
    error: Option<String>,
}

#[tauri::command]
fn delete_info(state: State<'_, Locker>, path: String) -> Reply<DeleteInfo> {
    let path = state.saved(&path)?;
    let (label, created, _, _) = describe(&path);
    let copies = state
        .store()?
        .copies_of(&path)
        .iter()
        .map(|copy| shown(copy))
        .collect();
    Ok(DeleteInfo {
        label,
        created,
        copies,
    })
}

fn outcomes<T>(results: Vec<(PathBuf, std::io::Result<T>)>) -> Vec<DeleteOutcome> {
    results
        .into_iter()
        .map(|(path, result)| DeleteOutcome {
            path: shown(&path),
            error: result.err().map(|error| error.to_string()),
        })
        .collect()
}

/// Moves a saved code into Recently deleted, which can be undone, so no word
/// has to be typed for it. If its code was just revealed, the code goes too.
#[tauri::command]
fn delete_saved(state: State<'_, Locker>, path: String) -> Reply<Vec<DeleteOutcome>> {
    let path = state.saved(&path)?;
    let revealed = lock(&state.revealed)
        .take()
        .filter(|(from, _)| *from == path)
        .map(|(_, code)| code);
    Ok(outcomes(
        state.store()?.move_to_bin(&path, revealed.as_deref()),
    ))
}

// ------------------------------------------------------ recently deleted

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeletedCode {
    /// The bin's name for it, which the UI hands back to restore or remove it.
    id: String,
    label: String,
    created: String,
    days_left: i64,
    /// Shown as it is: the code had been revealed before it was deleted.
    code: Option<String>,
    copies: Vec<String>,
}

#[tauri::command]
fn deleted_codes(state: State<'_, Locker>) -> Reply<Vec<DeletedCode>> {
    let store = state.store()?;
    let now = local_now();
    store.purge_expired(now);
    Ok(store
        .deleted()
        .into_iter()
        .map(|deleted| {
            let (label, created, _, _) = match deleted.copies.first() {
                Some(copy) => describe(copy),
                None => describe(Path::new("")),
            };
            DeletedCode {
                days_left: deleted.days_left(now),
                copies: deleted.copies.iter().map(|copy| shown(copy)).collect(),
                id: deleted.name,
                label,
                created,
                code: deleted.code,
            }
        })
        .collect())
}

#[tauri::command]
fn restore_deleted(state: State<'_, Locker>, id: String) -> Reply<Vec<DeleteOutcome>> {
    let deleted = state.binned(&id)?;
    Ok(outcomes(state.store()?.restore(&deleted)))
}

#[tauri::command]
fn delete_forever(
    state: State<'_, Locker>,
    id: String,
    confirmation: String,
) -> Reply<Vec<DeleteOutcome>> {
    if !confirmed(&confirmation, "delete") {
        return Err("Type delete to confirm.".into());
    }
    let deleted = state.binned(&id)?;
    Ok(outcomes(state.store()?.purge(&deleted)))
}

// ----------------------------------------------------------------- links

/// The page Donate opens. With None here, Settings has no Donate button.
const DONATE_URL: Option<&str> = Some("https://ko-fi.com/samwilliams123");

/// Where Send feedback writes to. With None here, Settings has no Send
/// feedback button.
const FEEDBACK_EMAIL: Option<&str> = Some("blindpasscodegen@gmail.com");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Links {
    donate: bool,
    /// Settings shows the address too, for a computer with no mail app.
    feedback_email: Option<&'static str>,
}

/// Which links Settings can offer.
#[tauri::command]
fn links() -> Links {
    Links {
        donate: DONATE_URL.is_some(),
        feedback_email: FEEDBACK_EMAIL,
    }
}

/// Hands one of the app's own links to the browser or the mail app. The UI
/// names the link and never gives an address, so the window can't send anyone
/// anywhere else. The app itself still connects to nothing.
#[tauri::command]
fn open_link(link: String) -> Reply<()> {
    let address = match link.as_str() {
        "donate" => DONATE_URL.map(str::to_string),
        "feedback" => FEEDBACK_EMAIL.map(|email| {
            format!(
                "mailto:{email}?subject=Blind%20Passcode%20Generator%20{}",
                env!("CARGO_PKG_VERSION")
            )
        }),
        _ => None,
    }
    .ok_or("That link isn't set up.")?;
    open::that_detached(address).map_err(|error| format!("Could not open it ({error})."))
}

// -------------------------------------------------------------- window

/// The window has no system title bar, so the page's own bar calls these.
#[tauri::command]
fn start_dragging(window: tauri::Window) -> Reply<()> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn minimize_window(window: tauri::Window) -> Reply<()> {
    window.minimize().map_err(|error| error.to_string())
}

#[tauri::command]
fn close_window(window: tauri::Window) -> Reply<()> {
    window.close().map_err(|error| error.to_string())
}

fn main() {
    tauri::Builder::default()
        .manage(Locker {
            store: Store::for_current_user(),
            setup: Mutex::new(None),
            starting: AtomicBool::new(false),
            revealing: Arc::new(AtomicBool::new(false)),
            cancel_reveal: Arc::new(AtomicBool::new(false)),
            revealed: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            keep_unfinished,
            discard_unfinished,
            describe_delay,
            start_setup,
            setup_view,
            setup_action,
            check_memory,
            back_up_finished,
            close_setup,
            lock_text,
            back_up_saved,
            reveal_info,
            reveal,
            cancel_reveal,
            delete_info,
            delete_saved,
            deleted_codes,
            restore_deleted,
            delete_forever,
            links,
            open_link,
            start_dragging,
            minimize_window,
            close_window,
        ])
        .run(tauri::generate_context!())
        .expect("Blind Passcode Generator could not start");
}
