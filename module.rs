// Account Switch: play as another signed-in Horizon account without quitting.
//
// The launcher signs several Microsoft accounts in and hands the game one
// session at start (`docs/microsoft-account.md`). `horizon-launcher account
// session <WHO>` renews another account's session for a game that is already
// running and prints the read-once session file it wrote. This mod asks for
// that file, hands it to `horizon_client::auth::set_current` in place of the
// session the game started with, leaves the server and joins it again: from the
// next login on the session service, the profile, the skins and the chat keys
// belong to the other account.
//
// What the player sees is Options > Mods' page of this mod (one row per
// signed-in account, like Zoom's settings) and a key that switches to the next
// account. The logic that can be reasoned about without the game (what the
// launcher prints, which account is next, the page's rows) is the mod's own
// crate, `horizon-account-switch-core`, with its tests.
use std::{
    path::PathBuf,
    process::Command,
    sync::{Mutex, MutexGuard},
};

use horizon_account_switch_core as logic;
use horizon_client::auth::AuthSession;

/// The mod's id: its settings folder, its row and its key's line.
pub const ID: &str = "account_switch";
pub const VERSION: &str = "0.1.0";
pub const DESCRIPTION: &str = "Play as another signed-in Horizon account without quitting: pick one here, \
     or press the key to switch to the next. The game leaves the server and joins it again as the other account.";
/// The key's name and its default: nothing vanilla uses.
pub const KEY: &str = "switch";
pub const DEFAULT_KEY: &str = "key.keyboard.f9";
/// Frames to let the title come back before joining again.
const RECONNECT_FRAMES: u32 = 2;

/// What the mod keeps between frames. One lock around all of it: the launcher
/// threads and the game's frame both take it, and it is never held across a
/// call into the game.
struct State {
    /// The launcher's accounts, in its order.
    accounts: Vec<logic::Account>,
    /// `account list` is running.
    listing: bool,
    /// The launcher answered: this account's session is ready to install.
    fetched: Option<AuthSession>,
    /// The account `fetched` belongs to.
    fetched_for: Option<String>,
    /// The account the game plays as now (none: the one it started with).
    playing: Option<String>,
    /// The server left for a switch: its address and the name to join with.
    back: Option<(String, String)>,
    /// Frames still to wait before joining again.
    wait: u32,
    /// The server joined last.
    last: Option<(String, String)>,
    /// The account list was asked for once.
    started: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    accounts: Vec::new(),
    listing: false,
    fetched: None,
    fetched_for: None,
    playing: None,
    back: None,
    wait: 0,
    last: None,
    started: false,
});

/// Messages for the next frame: the launcher threads can't reach the game.
static TELL: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn state() -> MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A line in the player's chat, on the title screen too.
fn tell(line: impl Into<String>) {
    TELL.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(line.into());
}

/// The launcher's executable: the launcher tells the game where it is
/// (`HORIZON_LAUNCHER_EXE`), so a switched launch finds the same launcher.
fn launcher() -> PathBuf {
    std::env::var_os("HORIZON_LAUNCHER_EXE")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("horizon-launcher"))
}

/// Runs the launcher and gives back what it printed; its own sentence when it
/// fails (`{"authentication": {…}}` arrives on stdout, other failures on
/// stderr).
fn run_launcher(args: &[&str]) -> Result<String, String> {
    let output = Command::new(launcher())
        .args(args)
        .output()
        .map_err(|error| format!("could not start the launcher: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if stderr.is_empty() {
        format!("the launcher failed ({})", output.status)
    } else {
        stderr
    })
}

/// Asks the launcher for its accounts, in the background: reading the keychain
/// and the sign-in of every account takes a moment.
fn refresh_accounts() {
    {
        let mut state = state();
        if state.listing {
            return;
        }
        state.listing = true;
    }
    std::thread::spawn(|| {
        let answer =
            run_launcher(&["account", "list", "--json"]).and_then(|text| logic::parse_list(&text));
        let mut state = state();
        state.listing = false;
        match answer {
            Ok(accounts) => state.accounts = accounts,
            Err(error) => log::warn!("Account Switch: the account list failed: {error}"),
        }
    });
}

/// Asks the launcher for `who`'s session, in the background: it renews the
/// Microsoft sign-in and the Minecraft token and writes the read-once session
/// file the game takes over.
fn start_switch(who: String) {
    std::thread::spawn(move || {
        let answer = run_launcher(&["account", "session", &who])
            .and_then(|text| logic::parse_session_file(&text))
            .and_then(|path| AuthSession::load(std::path::Path::new(&path)));
        match answer {
            Ok(session) => {
                let mut state = state();
                state.fetched = Some(session);
                state.fetched_for = Some(who);
            }
            Err(error) => {
                log::warn!("Account Switch: no session for {who}: {error}");
                tell(format!("Account Switch: {who} can't play: {error}"));
            }
        }
    });
}

/// The account the game plays as: the one switched to, else the one it
/// started with.
fn playing_name() -> Option<String> {
    if let Some(name) = state().playing.clone() {
        return Some(name);
    }
    horizon_client::auth::current().map(|session| session.name.clone())
}

/// A server is being joined: keep its address, for a switch to join it again.
pub(super) fn remember_server(address: &str, name: &str) {
    let mut state = state();
    state.last = Some((address.to_owned(), name.to_owned()));
}

/// Once a frame, on the window's own game: install a fetched session, leave the
/// server for it and join it again (`native_frame`'s hook).
pub(super) fn pump(game: &mut crate::game::Game<'_>) {
    if !game.native_window() {
        return;
    }
    for line in TELL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain(..)
    {
        game.system_message(horizon_client::text::Text::literal(line));
    }
    let first = {
        let mut state = state();
        let first = !state.started;
        state.started = true;
        first
    };
    if first {
        refresh_accounts();
    }
    // A session arrived: it is the account from now on.
    let fetched = {
        let mut state = state();
        match state.fetched.take() {
            Some(session) => Some((session, state.fetched_for.take())),
            None => None,
        }
    };
    if let Some((session, fetched_for)) = fetched {
        let who = fetched_for.unwrap_or_else(|| session.name.clone());
        // The whole of the game follows this session: the session service's
        // join, the profile, the skins. The chat keys are refetched for it.
        horizon_client::auth::set_current(session);
        horizon_client::auth::prepare_chat_keys(true);
        // A server is left and joined again, so the other account is on it now.
        // Single player (and its LAN guests) keep playing: the session only
        // matters for the next login, and quitting a world is the player's call.
        let in_server = game.remote.is_some() && game.integrated.is_none();
        {
            let mut state = state();
            state.playing = Some(who.clone());
            if in_server {
                state.back = state.last.clone();
                state.wait = RECONNECT_FRAMES;
            }
        }
        game.system_message(horizon_client::text::Text::literal(if in_server {
            format!("Account Switch: playing as {who}, joining the server again")
        } else {
            format!("Account Switch: playing as {who} from the next join on")
        }));
        if in_server {
            if let Err(error) = game.leave_server(None) {
                log::warn!("Account Switch: leaving the server failed: {error}");
            }
        }
    }
    // Back at the title: join the server the switch left.
    let back = {
        let mut state = state();
        match state.back.clone() {
            Some(_) if state.wait > 0 => {
                state.wait -= 1;
                None
            }
            Some(back) if game.remote.is_none() && game.title.is_some() => {
                state.back = None;
                Some(back)
            }
            _ => None,
        }
    };
    if let Some((address, name)) = back {
        game.system_message(horizon_client::text::Text::literal(format!(
            "Account Switch: joining {address} again"
        )));
        if let Err(error) = game.connect_server(&address, &name) {
            log::warn!("Account Switch: joining {address} failed: {error}");
        }
    }
}

/// The key: switch to the next signed-in account (`native_key`'s hook).
pub(super) fn on_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_window() {
        return;
    }
    if key != game.native_mod_key(ID, KEY, DEFAULT_KEY) {
        return;
    }
    let accounts = state().accounts.clone();
    let playing = playing_name();
    if accounts.is_empty() {
        refresh_accounts();
        tell("Account Switch: reading the launcher's accounts");
        return;
    }
    if let Some(index) = logic::next_account(&accounts, playing.as_deref()) {
        let who = accounts[index].name.clone();
        tell(format!("Account Switch: switching to {who}"));
        start_switch(who);
    }
}

/// The mod's row in Options > Mods (`native_mods`' hook).
pub(super) fn listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: ID.to_owned(),
        title: format!("Account Switch {VERSION}"),
        description: DESCRIPTION.to_owned(),
        keys: vec![[
            KEY.to_owned(),
            "Switch to the next account".to_owned(),
            DEFAULT_KEY.to_owned(),
        ]],
    });
}

/// The mod's settings page: a row per signed-in account
/// (`native_settings`' hook).
pub(super) fn page(mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    let (accounts, listing) = {
        let state = state();
        (state.accounts.clone(), state.listing)
    };
    let rows = logic::labels(&accounts, playing_name().as_deref(), listing);
    if !listing && accounts.is_empty() {
        // The page is open: keep the list fresh without waiting for a click.
        refresh_accounts();
    }
    buttons.extend(rows);
}

/// A click on the page (`native_settings_click`'s hook).
pub(super) fn clicked(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    let (accounts, listing) = {
        let state = state();
        (state.accounts.clone(), state.listing)
    };
    let playing = playing_name();
    let rows = logic::labels(&accounts, playing.as_deref(), listing);
    match logic::click(rows.len(), accounts.len(), button) {
        logic::Click::Refresh => {
            refresh_accounts();
            game.system_message(horizon_client::text::Text::literal(
                "Account Switch: reading the launcher's accounts",
            ));
        }
        logic::Click::Account(index) => {
            let Some(account) = accounts.get(index) else {
                return;
            };
            if playing.is_some_and(|name| name.eq_ignore_ascii_case(&account.name)) {
                return;
            }
            let who = account.name.clone();
            start_switch(who.clone());
            game.system_message(horizon_client::text::Text::literal(format!(
                "Account Switch: switching to {who}"
            )));
        }
        logic::Click::Nothing => {}
    }
}
