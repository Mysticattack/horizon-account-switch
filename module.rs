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
// The switch key (F9) opens the mod's own screen: one row per signed-in
// account, a row that starts a Microsoft sign-in for a new one, and the code to
// enter. Options > Mods carries the same rows as the mod's settings page.
// The logic that can be reasoned about without the game (what the launcher
// prints, the next account, the screen's geometry) is the mod's own crate,
// `horizon-account-switch-core`, with its tests.
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Mutex, MutexGuard},
};

use horizon_account_switch_core as logic;
use horizon_client::auth::AuthSession;
use horizon_client::clipboard::Clipboard;

/// The mod's id: its settings folder, its row, its key's line and its screen.
pub const ID: &str = "account_switch";
pub const VERSION: &str = "0.3.0";
pub const DESCRIPTION: &str =
    "Play as another signed-in Horizon account: press the key, pick one, or add an account with your \
     Microsoft sign-in (the code is copied and the page opens). The game leaves the server and joins it \
     again as the other account.";
/// The key's name and its default: nothing vanilla uses.
pub const KEY: &str = "switch";
pub const DEFAULT_KEY: &str = "key.keyboard.f9";
/// The screen's title.
const TITLE: &str = "Account Switch";
/// Frames to let the title come back before joining again.
const RECONNECT_FRAMES: u32 = 2;

// The screen's colours, ARGB as the game's `fill` and `text` take them.
const SHADE: u32 = 0x60_00_00_00;
const PANEL: u32 = 0xF2_16_16_1C;
const BORDER: u32 = 0xFF_4A_4A_58;
const TITLE_TEXT: u32 = 0xFF_FF_FF_FF;
const TEXT: u32 = 0xFF_E6_E6_E6;
const MUTED: u32 = 0xFF_9A_9A_A6;
const HERE: u32 = 0xFF_6E_D8_6E;
const HOVER: u32 = 0x40_FF_FF_FF;
const CODE: u32 = 0xFF_FF_E0_70;

/// A Microsoft sign-in the screen started: the code and where to enter it.
#[derive(Clone, Debug)]
pub(crate) struct SignIn {
    /// The account signed in again, or none for a new one.
    pub(crate) target: Option<String>,
    pub(crate) link: String,
    pub(crate) code: String,
}

/// What the mod keeps between frames. One lock around all of it: the launcher
/// threads, the screen and the game's frame all take it, and it is never held
/// across a call into the game.
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
    /// A sign-in is running, and its code once the launcher printed one.
    sign_in: Option<SignIn>,
    sign_in_running: bool,
    /// The screen's one line about what happened last.
    status: Option<String>,
    /// A code to put on the clipboard on the next frame (the screen can't).
    copy: Option<String>,
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
    sign_in: None,
    sign_in_running: false,
    status: None,
    copy: None,
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

fn set_status(line: impl Into<String>) {
    state().status = Some(line.into());
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

/// Opens `link` in the player's browser: the sign-in page, so the code can be
/// pasted there without typing the address.
fn open_page(link: &str) {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    match command
        .arg(link)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => {}
        Err(error) => log::warn!("Account Switch: could not open {link}: {error}"),
    }
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
            Err(error) => {
                log::warn!("Account Switch: the account list failed: {error}");
                state.status = Some(error);
            }
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
                state.status = Some(format!("switching to {who}…"));
                state.fetched_for = Some(who);
            }
            Err(error) => {
                log::warn!("Account Switch: no session for {who}: {error}");
                tell(format!("Account Switch: {who} can't play: {error}"));
                set_status(format!("{who}: {error}"));
            }
        }
    });
}

/// Starts a Microsoft sign-in: a new account (`None`), or `who` again.
fn add_account(target: Option<String>) {
    {
        let mut state = state();
        if state.sign_in_running {
            return;
        }
        state.sign_in_running = true;
        state.sign_in = None;
        state.status = Some("asking Microsoft for a code…".to_owned());
    }
    std::thread::spawn(move || {
        let mut command = Command::new(launcher());
        command.args(["account", "sign-in"]);
        if let Some(who) = &target {
            command.args(["--for", who.as_str()]);
        }
        let mut child = match command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
            Ok(child) => child,
            Err(error) => {
                finish_sign_in(format!("could not start the launcher: {error}"));
                return;
            }
        };
        // The CLI prints "To sign in, open <link> in a browser and enter the
        // code <CODE>" on stderr, then waits for the sign-in to finish.
        if let Some(errors) = child.stderr.take() {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                if let Some((link, code)) = logic::parse_sign_in(&line) {
                    // The page opens here, the code goes to the clipboard on
                    // the next frame (this thread can't reach the game).
                    open_page(&link);
                    {
                        let mut state = state();
                        state.sign_in = Some(SignIn {
                            target: target.clone(),
                            link,
                            code: code.clone(),
                        });
                        state.copy = Some(code);
                        state.status =
                            Some("the code is copied: paste it on the page, this list follows".to_owned());
                    }
                }
            }
        }
        let signed_in = child.wait().map(|status| status.success()).unwrap_or(false);
        if signed_in {
            refresh_accounts();
            finish_sign_in("signed in".to_owned());
        } else {
            finish_sign_in("the sign-in did not finish".to_owned());
        }
    });
}

fn finish_sign_in(status: String) {
    let mut state = state();
    state.sign_in_running = false;
    state.sign_in = None;
    state.status = Some(status);
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
    // A sign-in's code goes to the system clipboard: the screen is `&self`
    // and can't, this frame can.
    let copy = state().copy.take();
    if let Some(code) = copy {
        game.clipboard.set(&code);
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
            state.status = Some(format!("playing as {who}"));
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

/// One row of the screen: what a click on it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Ask the launcher for its accounts again.
    Refresh,
    /// Switch to the account at this index of the list.
    Switch(usize),
    /// Start a Microsoft sign-in for a new account.
    Add,
    /// Put the running sign-in's code on the clipboard again.
    Copy,
}

/// What the screen shows, read in one go.
pub(crate) struct View {
    pub(crate) accounts: Vec<logic::Account>,
    pub(crate) playing: Option<String>,
    listing: bool,
    pub(crate) sign_in: Option<SignIn>,
    status: Option<String>,
}

impl View {
    /// The screen's rows, top to bottom: the actions and their labels.
    pub(crate) fn rows(&self) -> Vec<(Action, String)> {
        let mut rows = vec![(
            Action::Refresh,
            if self.listing {
                "Loading accounts…".to_owned()
            } else {
                "Refresh list".to_owned()
            },
        )];
        for (index, account) in self.accounts.iter().enumerate() {
            let here = self
                .playing
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&account.name));
            let ready = account.state.is_empty() || account.state == "ready";
            let label = if here {
                format!("{} — playing now", account.name)
            } else if ready {
                format!("Play as {}", account.name)
            } else {
                format!("Sign in again: {}", account.name)
            };
            rows.push((Action::Switch(index), label));
        }
        rows.push((Action::Add, "+ Add account (Microsoft sign-in)".to_owned()));
        if let Some(sign_in) = &self.sign_in {
            rows.push((Action::Copy, format!("Copy code {}", sign_in.code)));
        }
        rows
    }
}

fn view() -> View {
    let (accounts, listing, sign_in, status) = {
        let state = state();
        (
            state.accounts.clone(),
            state.listing,
            state.sign_in.clone(),
            state.status.clone(),
        )
    };
    View {
        accounts,
        playing: playing_name(),
        listing,
        sign_in,
        status,
    }
}

/// Opens the switch screen (`native_mod_screen_open`, API 111): the key, or
/// Options > Mods' page could.
pub(super) fn open(game: &mut crate::game::Game<'_>) {
    if game.native_mod_screen_open(ID) {
        refresh_accounts();
    }
}

/// Draws the screen over the world (`native_screen_render_post`, API 78).
pub(super) fn draw(
    game: &crate::game::Game<'_>,
    batch: &mut crate::hud::HudBatch,
    screen: &crate::mod_api::NativeScreenView<'_>,
) {
    if game.native_mod_screen() != Some(ID) {
        return;
    }
    let view = view();
    let rows = view.rows();
    let window = [screen.rect[0] + screen.rect[2], screen.rect[1] + screen.rect[3]];
    let panel = logic::panel(window, rows.len());
    let host: &dyn crate::mod_api::Host = game;
    host.fill(batch, [0.0, 0.0, window[0], window[1]], SHADE);
    host.fill(batch, panel, PANEL);
    // The border: four thin fills, the panel's edge.
    let [x, y, width, height] = panel;
    host.fill(batch, [x, y, width, 1.0], BORDER);
    host.fill(batch, [x, y + height - 1.0, width, 1.0], BORDER);
    host.fill(batch, [x, y, 1.0, height], BORDER);
    host.fill(batch, [x + width - 1.0, y, 1.0, height], BORDER);
    host.text(batch, TITLE, [x + 8.0, y + 8.0], TITLE_TEXT, true);
    let hovered = logic::hit(panel, rows.len(), screen.mouse);
    for (index, (action, label)) in rows.iter().enumerate() {
        let rect = logic::row(panel, index);
        if hovered == Some(index) {
            host.fill(batch, rect, HOVER);
        }
        let here = matches!(action, Action::Switch(at)
            if view.accounts.get(*at).is_some_and(|account| view
                .playing
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&account.name))));
        let rgb = if here { HERE } else { TEXT };
        host.text(batch, label, [rect[0] + 4.0, rect[1] + 6.0], rgb, true);
    }
    host.text(
        batch,
        "Esc closes",
        [x + 8.0, y + height - logic::FOOTER / 2.0 - 4.0],
        MUTED,
        true,
    );
    // Below the panel: the code of a running sign-in, or what happened last.
    let mut line = y + height + 6.0;
    if let Some(sign_in) = &view.sign_in {
        host.text(batch, &format!("Code: {}", sign_in.code), [x + 8.0, line], CODE, true);
        line += 12.0;
        host.text(batch, &sign_in.link, [x + 8.0, line], TEXT, true);
        line += 12.0;
        host.text(
            batch,
            if sign_in.target.is_some() {
                "enter it to sign that account in again"
            } else {
                "enter it to add that account"
            },
            [x + 8.0, line],
            MUTED,
            true,
        );
    } else if let Some(status) = &view.status {
        host.text(batch, status, [x + 8.0, line], MUTED, true);
    }
}

/// A click over the screen (`native_screen_mouse_button_pressed_pre`, API 80):
/// a row acts, and the press goes no further.
pub(super) fn clicked(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    // Left button only; a gamepad or keyboard click never gets here.
    if button != 0 || game.native_mod_screen() != Some(ID) {
        return;
    }
    let view = view();
    let rows = view.rows();
    let panel = logic::panel([screen.rect[2], screen.rect[3]], rows.len());
    let Some(index) = logic::hit(panel, rows.len(), screen.mouse) else {
        return;
    };
    let Some((action, label)) = rows.get(index).cloned() else {
        return;
    };
    *canceled = true;
    match action {
        Action::Refresh => {
            refresh_accounts();
            set_status("reading the launcher's accounts…");
        }
        Action::Switch(at) => {
            let Some(account) = view.accounts.get(at) else {
                return;
            };
            if view
                .playing
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&account.name))
            {
                return;
            }
            let ready = account.state.is_empty() || account.state == "ready";
            if ready {
                start_switch(account.name.clone());
            } else {
                add_account(Some(account.name.clone()));
            }
        }
        Action::Add => add_account(None),
        Action::Copy => {
            if let Some(sign_in) = view.sign_in.clone() {
                state().copy = Some(sign_in.code);
                set_status("code copied");
            }
        }
    }
    let _ = label;
}

/// The key: open (or close) the switch screen (`native_key`'s hook).
pub(super) fn on_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_window() {
        return;
    }
    if key != game.native_mod_key(ID, KEY, DEFAULT_KEY) {
        return;
    }
    if game.native_mod_screen() == Some(ID) {
        game.native_mod_screen_close(ID);
        return;
    }
    open(game);
}

/// The mod's row in Options > Mods (`native_mods`' hook).
pub(super) fn listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: ID.to_owned(),
        title: format!("Account Switch {VERSION}"),
        description: DESCRIPTION.to_owned(),
        keys: vec![[
            KEY.to_owned(),
            "Open the account switcher".to_owned(),
            DEFAULT_KEY.to_owned(),
        ]],
    });
}

/// The mod's settings page: the same rows as the screen
/// (`native_settings`' hook).
pub(super) fn page(mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    let view = view();
    if !view.listing && view.accounts.is_empty() {
        // The page is open: keep the list fresh without waiting for a click.
        refresh_accounts();
    }
    buttons.extend(view.rows().into_iter().map(|(_, label)| label));
}

/// A click on the page (`native_settings_click`'s hook).
pub(super) fn clicked_page(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    let view = view();
    let rows = view.rows();
    let Some((action, _)) = rows.get(button).cloned() else {
        return;
    };
    match action {
        Action::Refresh => {
            refresh_accounts();
            game.system_message(horizon_client::text::Text::literal(
                "Account Switch: reading the launcher's accounts",
            ));
        }
        Action::Switch(at) => {
            let Some(account) = view.accounts.get(at) else {
                return;
            };
            if view
                .playing
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&account.name))
            {
                return;
            }
            let ready = account.state.is_empty() || account.state == "ready";
            if ready {
                start_switch(account.name.clone());
                game.system_message(horizon_client::text::Text::literal(format!(
                    "Account Switch: switching to {}",
                    account.name
                )));
            } else {
                add_account(Some(account.name.clone()));
            }
        }
        Action::Add => add_account(None),
        Action::Copy => {
            if let Some(sign_in) = view.sign_in.clone() {
                state().copy = Some(sign_in.code);
            }
        }
    }
}
