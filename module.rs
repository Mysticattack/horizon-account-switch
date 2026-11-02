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
pub const VERSION: &str = "0.7.0";
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

// The screen's colours, ARGB as the game's `fill` and `text` take them:
// vanilla's tooltip background, and the text colours of its buttons.
const SHADE: u32 = 0x80_10_10_10;
const TITLE_TEXT: u32 = 0xFF_FF_FF_FF;
const MUTED: u32 = 0xFF_A0_A0_A0;
const HERE: u32 = 0xFF_6E_D8_6E;
/// A button's label under the pointer, as vanilla colours it.
const HOVER_TEXT: u32 = 0xFF_FF_FF_A0;
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
    /// A switch in flight: the account, and when it started. The screen shows
    /// it and takes no second request meanwhile.
    busy: Option<(String, std::time::Instant)>,
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
    busy: None,
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
    {
        let mut state = state();
        if let Some((other, since)) = &state.busy {
            if since.elapsed().as_secs() < 60 {
                state.status = Some(format!("still switching to {other}…"));
                return;
            }
        }
        state.busy = Some((who.clone(), std::time::Instant::now()));
        state.status = Some(format!("switching to {who}…"));
    }
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
                let mut state = state();
                state.busy = None;
                state.status = Some(format!("{who}: {error}"));
            }
        }
    });
}

/// Starts a Microsoft sign-in: a new account (`None`), or `who` again.
fn add_account(target: Option<String>) {
    let known: Vec<String> = {
        let mut state = state();
        if state.sign_in_running {
            state.status = Some("a sign-in is already running".to_owned());
            return;
        }
        state.sign_in_running = true;
        state.sign_in = None;
        state.status = Some("asking Microsoft for a code…".to_owned());
        state.accounts.iter().map(|account| account.name.clone()).collect()
    };
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
        let finished = child.wait();
        let mut said = String::new();
        if let Some(mut out) = child.stdout.take() {
            use std::io::Read;
            let _ = out.read_to_string(&mut said);
        }
        let signed_in = finished.map(|status| status.success()).unwrap_or(false);
        if signed_in {
            refresh_accounts();
            let name = logic::signed_in_name(&said);
            let already = name.as_ref().is_some_and(|name| {
                known.iter().any(|known| known.eq_ignore_ascii_case(name))
            });
            finish_sign_in(match name {
                Some(name) if already => {
                    tell(format!("Account Switch: {name} is already signed in"));
                    format!("{name} is already signed in")
                }
                Some(name) => {
                    tell(format!("Account Switch: {name} signed in"));
                    format!("{name} signed in")
                }
                None => "signed in".to_owned(),
            });
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
            if in_server {
                // The reconnect below ends the switch; until then the screen
                // keeps saying what it is doing.
                state.status = Some(format!("joining the server as {who}…"));
                state.back = state.last.clone();
                state.wait = RECONNECT_FRAMES;
            } else {
                state.busy = None;
                state.status = Some(format!("playing as {who} from the next join on"));
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
        let failure = game.connect_server(&address, &name).err();
        {
            let mut state = state();
            state.busy = None;
            state.status = Some(match &failure {
                Some(error) => format!("joining {address} failed: {error}"),
                None => format!("joined {address} again"),
            });
        }
    }
    // A switch that never finished (a hung launcher) lets go of the screen.
    {
        let mut state = state();
        let hung = state
            .busy
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed().as_secs() > 60);
        if hung {
            state.busy = None;
            state.status = Some("the switch took too long".to_owned());
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
    /// A switch is in flight: the row only says so.
    Busy,
}

/// What the screen shows, read in one go.
pub(crate) struct View {
    pub(crate) accounts: Vec<logic::Account>,
    pub(crate) playing: Option<String>,
    listing: bool,
    pub(crate) sign_in: Option<SignIn>,
    status: Option<String>,
    /// A switch in flight: the account, and how long it has been running.
    pub(crate) busy: Option<(String, f32)>,
}

impl View {
    /// The screen's rows, top to bottom: the actions and their labels.
    pub(crate) fn rows(&self) -> Vec<(Action, String)> {
        if let Some((who, _)) = &self.busy {
            return vec![(Action::Busy, format!("Switching to {who}…"))];
        }
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
    let busy = state()
        .busy
        .as_ref()
        .map(|(who, since)| (who.clone(), since.elapsed().as_secs_f32()));
    View {
        accounts,
        playing: playing_name(),
        listing,
        sign_in,
        status,
        busy,
    }
}

/// Opens the switch screen (`native_mod_screen_open`, API 111): the key, or
/// Options > Mods' page could.
pub(super) fn open(game: &mut crate::game::Game<'_>) {
    if game.native_mod_screen_open(ID) {
        refresh_accounts();
    }
}

// Vanilla's own menu art: the dark dirt every menu shows over a world, its
// separators, and the buttons.
const BACKGROUND: &str = "gui/inworld_menu_background";
const HEADER_SEPARATOR: &str = "gui/inworld_header_separator";
const FOOTER_SEPARATOR: &str = "gui/inworld_footer_separator";
const BUTTON: &str = "gui/sprites/widget/button";
const BUTTON_HOVER: &str = "gui/sprites/widget/button_highlighted";
const BUTTON_OFF: &str = "gui/sprites/widget/button_disabled";
/// The tiles' size: the game draws the menu background at 32 GUI pixels.
const TILE: f32 = 32.0;
/// A vanilla button's texture is 200x20 (`gui/sprites/widget/button`).
const BUTTON_WIDTH: f32 = 200.0;
const BUTTON_HEIGHT: f32 = 20.0;

/// Draws `sprite` as the game draws a button: the texture's two halves, each
/// stretched to half the width, so the rounded ends keep their shape.
fn draw_button(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    rect: [f32; 4],
    sprite: &str,
) {
    let [x, y, width, height] = rect;
    let half = (width / 2.0).floor();
    let middle = BUTTON_WIDTH / 2.0;
    host.sprite(batch, sprite, [0.0, 0.0, middle, BUTTON_HEIGHT], [x, y, half, height]);
    host.sprite(
        batch,
        sprite,
        [middle, 0.0, middle, BUTTON_HEIGHT],
        [x + half, y, width - half, height],
    );
}

/// Repeats `sprite` over `rect` in `cell`-sized tiles, the last row and column
/// cut to fit, as vanilla tiles its menu background and separators. False when
/// the atlas has no such sprite.
fn draw_tiled(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    game: &crate::game::Game<'_>,
    sprite: &str,
    rect: [f32; 4],
    cell: [f32; 2],
) -> bool {
    let Some(size) = game.native_sprite_size(sprite) else {
        return false;
    };
    let [x, y, width, height] = rect;
    if width <= 0.0 || height <= 0.0 || size[0] <= 0.0 || size[1] <= 0.0 {
        return false;
    }
    let columns = (width / cell[0]).ceil() as i32;
    let rows = (height / cell[1]).ceil() as i32;
    for row in 0..rows {
        for column in 0..columns {
            let left = column as f32 * cell[0];
            let top = row as f32 * cell[1];
            let used_x = (width - left).min(cell[0]);
            let used_y = (height - top).min(cell[1]);
            if used_x <= 0.0 || used_y <= 0.0 {
                continue;
            }
            host.sprite(
                batch,
                sprite,
                [0.0, 0.0, size[0] * used_x / cell[0], size[1] * used_y / cell[1]],
                [x + left, y + top, used_x, used_y],
            );
        }
    }
    true
}

/// The button column in GUI pixels: the same rectangle for drawing and clicks.
fn panel_of(screen: &crate::mod_api::NativeScreenView<'_>, rows: usize) -> [f32; 4] {
    logic::panel(
        [screen.rect[0] + screen.rect[2], screen.rect[1] + screen.rect[3]],
        rows,
    )
}

/// Draws the screen over the world (`native_screen_render_post`, API 78) the
/// way vanilla draws a menu: the tiled dark dirt, the title over a header
/// separator, one button per row, a footer separator, and the hint in the
/// screen's bottom corner.
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
    let host: &dyn crate::mod_api::Host = game;
    let window = [screen.rect[0] + screen.rect[2], screen.rect[1] + screen.rect[3]];
    let whole = [0.0, 0.0, window[0], window[1]];
    if !draw_tiled(host, batch, game, BACKGROUND, whole, [TILE, TILE]) {
        host.fill(batch, whole, SHADE);
    }
    let panel = panel_of(screen, rows.len());
    let [x, y, width, height] = panel;
    // The title, and the header separator under it, as on every vanilla menu.
    let title = game.native_text_width(TITLE);
    host.text(
        batch,
        TITLE,
        [x + (width - title) / 2.0, y - 26.0],
        TITLE_TEXT,
        true,
    );
    draw_tiled(
        host,
        batch,
        game,
        HEADER_SEPARATOR,
        [0.0, y - 8.0, window[0], 2.0],
        [TILE, 2.0],
    );
    let hovered = logic::hit(panel, rows.len(), screen.mouse);
    for (index, (action, label)) in rows.iter().enumerate() {
        let rect = logic::row(panel, index);
        let here = matches!(action, Action::Switch(at)
            if view.accounts.get(*at).is_some_and(|account| view
                .playing
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&account.name))));
        let sprite = if matches!(action, Action::Busy) {
            BUTTON_OFF
        } else if hovered == Some(index) {
            BUTTON_HOVER
        } else {
            BUTTON
        };
        draw_button(host, batch, rect, sprite);
        // Vanilla turns a button's label yellow under the pointer.
        let text = if matches!(action, Action::Busy) {
            MUTED
        } else if here {
            HERE
        } else if hovered == Some(index) {
            HOVER_TEXT
        } else {
            TITLE_TEXT
        };
        let width_of = game.native_text_width(label);
        host.text(
            batch,
            label,
            [rect[0] + (rect[2] - width_of) / 2.0, rect[1] + 6.0],
            text,
            true,
        );
    }
    // A switch in flight: a bar inside its button.
    if let Some((_, elapsed)) = &view.busy {
        let rect = logic::row(panel, 0);
        let fraction = 0.15 + 0.85 * (0.5 - 0.5 * (elapsed * 3.0).cos());
        host.fill(
            batch,
            [
                rect[0] + 4.0,
                rect[1] + rect[3] - 5.0,
                (rect[2] - 8.0) * fraction,
                2.0,
            ],
            HERE,
        );
    }
    // The footer separator, then what happened last or a sign-in's code.
    let footer = y + height + 12.0;
    draw_tiled(
        host,
        batch,
        game,
        FOOTER_SEPARATOR,
        [0.0, footer, window[0], 2.0],
        [TILE, 2.0],
    );
    let mut line = footer + 8.0;
    if let Some(sign_in) = &view.sign_in {
        draw_line(host, batch, game, panel, &format!("Code: {}", sign_in.code), line, CODE);
        line += 12.0;
        draw_line(host, batch, game, panel, &sign_in.link, line, TITLE_TEXT);
        line += 12.0;
        draw_line(
            host,
            batch,
            game,
            panel,
            if sign_in.target.is_some() {
                "enter it to sign that account in again"
            } else {
                "enter it to add that account"
            },
            line,
            MUTED,
        );
    } else if let Some(status) = &view.status {
        draw_line(host, batch, game, panel, status, line, MUTED);
    }
    let hint = "Esc closes · click a row to switch";
    let hint_width = game.native_text_width(hint);
    host.text(
        batch,
        hint,
        [x + (width - hint_width) / 2.0, window[1] - 18.0],
        MUTED,
        true,
    );
}

/// A line of text under the column, centred with it.
fn draw_line(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    game: &crate::game::Game<'_>,
    panel: [f32; 4],
    text: &str,
    y: f32,
    rgb: u32,
) {
    let width = game.native_text_width(text);
    host.text(batch, text, [panel[0] + (panel[2] - width) / 2.0, y], rgb, true);
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
    let panel = panel_of(screen, rows.len());
    let Some(index) = logic::hit(panel, rows.len(), screen.mouse) else {
        return;
    };
    let Some((action, label)) = rows.get(index).cloned() else {
        return;
    };
    *canceled = true;
    if let Some((who, _)) = &view.busy {
        set_status(format!("still switching to {who}…"));
        return;
    }
    match action {
        Action::Busy => {}
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
                let line = format!("{} is already signed in and playing", account.name);
                set_status(line.clone());
                tell(format!("Account Switch: {line}"));
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
    if let Some((who, _)) = &view.busy {
        game.system_message(horizon_client::text::Text::literal(format!(
            "Account Switch: still switching to {who}"
        )));
        return;
    }
    match action {
        Action::Busy => {}
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
                game.system_message(horizon_client::text::Text::literal(format!(
                    "Account Switch: {} is already signed in and playing",
                    account.name
                )));
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
