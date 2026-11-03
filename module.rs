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
    collections::{BTreeMap, VecDeque},
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
pub const VERSION: &str = "0.22.0";
pub const DESCRIPTION: &str =
    "Play as another signed-in Horizon account: press the key, pick one, or add an account with your \
     Microsoft sign-in (the code is copied and the page opens). The game leaves the server and joins it \
     again as the other account. Hold the zoom key to zoom in (the wheel changes how far), and press the map \
     key for the world map.";
/// The key's name and its default: nothing vanilla uses.
pub const KEY: &str = "switch";
pub const DEFAULT_KEY: &str = "key.keyboard.f9";
/// The zoom key: held, it zooms in; the wheel changes how far.
pub const ZOOM_KEY: &str = "zoom";
pub const ZOOM_DEFAULT: &str = "key.keyboard.c";
/// The map key: it opens the minimap into the big map and back.
pub const MAP_KEY: &str = "map";
pub const MAP_DEFAULT: &str = "key.keyboard.m";

/// The minimap's zoom keys: blocks per cell, as the Minimap has it.
pub const MINI_OUT_KEY: &str = "minimap_zoom_out";
pub const MINI_OUT_DEFAULT: &str = "key.keyboard.bracketleft";
pub const MINI_IN_KEY: &str = "minimap_zoom_in";
pub const MINI_IN_DEFAULT: &str = "key.keyboard.bracketright";

/// The waypoint key: a marker where the player stands, as the World Map has it.
pub const WAYPOINT_KEY: &str = "waypoint";
pub const WAYPOINT_DEFAULT: &str = "key.keyboard.b";
/// The screen's title.
const TITLE: &str = "Account Switch";
/// Frames to let the title come back before joining again.
const RECONNECT_FRAMES: u32 = 2;

// The screen's colours, ARGB as the game's `fill` and `text` take them:
// vanilla's tooltip background, and the text colours of its buttons.
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
    /// The first row the screen shows: the list scrolls when the accounts
    /// don't fit.
    scroll: usize,
    /// Held zoom factor while the zoom key is down.
    zoom: Option<f32>,
    /// The map key was pressed: the screen shows the world map.
    map_open: bool,
    /// Blocks per world map cell.
    map_zoom: u32,
    /// The minimap's zoom: blocks per cell.
    mini_zoom: u32,
    /// Every block the player has seen, one colour each, in tiles of
    /// `logic::TILE` blocks: the map keeps showing terrain after its chunk
    /// unloads, as Xaero's does. `tile_order` says which tile goes first when
    /// the map is full.
    tiles: Tiles,
    tile_order: VecDeque<(i32, i32)>,
    /// The other worlds' maps, parked while another one is played: the nether's
    /// map is not the overworld's, and a server's not the next server's.
    parked: BTreeMap<String, (Tiles, VecDeque<(i32, i32)>)>,
    /// The world the map in `tiles` belongs to.
    active: String,
    /// Where the last exploration pass ran, and how often it may run, in
    /// milliseconds (42 is 24 times a second).
    explored_at: Option<((i32, i32), std::time::Instant)>,
    mini_every: u128,
    /// How far the world map is dragged from the player, in blocks.
    pan: (i32, i32),
    /// The waypoints the world map shows.
    waypoints: Vec<(i32, i32)>,
    /// The build said what it found, once.
    map_said: bool,
    /// What the last build found, for the map screen's own line.
    map_debug: String,
    /// The minimap's corner, as `logic::corner` counts them.
    corner: usize,
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
    scroll: 0,
    zoom: None,
    map_open: false,
    map_zoom: 1,
    mini_zoom: 1,
    tiles: BTreeMap::new(),
    tile_order: VecDeque::new(),
    parked: BTreeMap::new(),
    active: String::new(),
    explored_at: None,
    mini_every: 42,
    pan: (0, 0),
    waypoints: Vec::new(),
    map_said: false,
    map_debug: String::new(),
    corner: 1,
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
    // The world map: built while it is open or the minimap shows it, and
    // forgotten when its screen went away (Escape).
    {
        let open = state().map_open;
        if open && game.native_mod_screen() != Some(ID) {
            state().map_open = false;
        }
    }
    if game.native_playing() {
        explore(game);
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
    /// The first row the screen shows.
    pub(crate) scroll: usize,
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
    let scroll = state().scroll;
    View {
        accounts,
        playing: playing_name(),
        listing,
        sign_in,
        status,
        busy,
        scroll,
    }
}

/// Opens the switch screen (`native_mod_screen_open`, API 111): the key, or
/// Options > Mods' page could.
pub(super) fn open(game: &mut crate::game::Game<'_>) {
    if game.native_mod_screen_open(ID) {
        refresh_accounts();
    }
}

// The screen's own look: a dark slate panel, rows that match it, and a scroll
// lane — plain fills, so nothing depends on a resource pack's sprites.
const VEIL: u32 = 0xB4_00_00_00;
const PANEL: u32 = 0xF2_16_16_1E;
const HEADER_FILL: u32 = 0xFF_1E_1E_28;
const BORDER: u32 = 0xFF_3E_3E_52;
const ROW_FILL: u32 = 0xF2_26_26_32;
const ROW_FILL_HOVER: u32 = 0xF2_34_34_46;
const ROW_FILL_ACTIVE: u32 = 0xF2_1C_33_24;
const ROW_FILL_BUSY: u32 = 0xF2_22_22_2A;
const ROW_BORDER: u32 = 0xFF_3E_3E_52;
const ROW_BORDER_HOVER: u32 = 0xFF_8A_8A_A8;
const ROW_BORDER_ACTIVE: u32 = 0xFF_46_8A_58;
const ROW_BORDER_BUSY: u32 = 0xFF_45_45_50;
const SCROLL_TRACK: u32 = 0x30_FF_FF_FF;
const SCROLL_THUMB: u32 = 0xFF_7A_7A_98;

/// A filled rectangle with a one-pixel border, the shape of everything here.
fn draw_box(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    rect: [f32; 4],
    fill: u32,
    border: u32,
) {
    let [x, y, width, height] = rect;
    host.fill(batch, rect, fill);
    host.fill(batch, [x, y, width, 1.0], border);
    host.fill(batch, [x, y + height - 1.0, width, 1.0], border);
    host.fill(batch, [x, y, 1.0, height], border);
    host.fill(batch, [x + width - 1.0, y, 1.0, height], border);
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
    if state().map_open {
        draw_map(game, batch, screen);
        return;
    }
    let view = view();
    let rows = view.rows();
    let host: &dyn crate::mod_api::Host = game;
    let window = [screen.rect[0] + screen.rect[2], screen.rect[1] + screen.rect[3]];
    host.fill(batch, [0.0, 0.0, window[0], window[1]], VEIL);
    let visible = logic::visible_rows(window).min(rows.len().max(1));
    let (first, count) = logic::window_of(view.scroll, rows.len(), visible);
    let panel = logic::panel(window, visible.max(1));
    let [x, y, width, height] = panel;
    draw_box(host, batch, panel, PANEL, BORDER);
    // The title bar, a shade lighter, with a line under it.
    host.fill(
        batch,
        [x + 1.0, y + 1.0, width - 2.0, logic::HEADER - 1.0],
        HEADER_FILL,
    );
    host.fill(
        batch,
        [x + 1.0, y + logic::HEADER, width - 2.0, 1.0],
        BORDER,
    );
    let title = game.native_text_width(TITLE);
    host.text(
        batch,
        TITLE,
        [x + (width - title) / 2.0, y + 8.0],
        TITLE_TEXT,
        true,
    );
    let hovered = logic::hit(panel, visible, screen.mouse);
    for (index, (action, label)) in rows[first..first + count].iter().enumerate() {
        let rect = logic::row(panel, index);
        let here = matches!(action, Action::Switch(at)
            if view.accounts.get(*at).is_some_and(|account| view
                .playing
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&account.name))));
        let busy = matches!(action, Action::Busy);
        let (fill, border) = if busy {
            (ROW_FILL_BUSY, ROW_BORDER_BUSY)
        } else if here {
            (ROW_FILL_ACTIVE, ROW_BORDER_ACTIVE)
        } else if hovered == Some(index) {
            (ROW_FILL_HOVER, ROW_BORDER_HOVER)
        } else {
            (ROW_FILL, ROW_BORDER)
        };
        draw_box(host, batch, rect, fill, border);
        // Vanilla colours a button's label under the pointer yellow.
        let text = if busy {
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
    // A switch in flight: a bar inside its row.
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
    // The scroll lane, and its thumb when there is more than one page.
    if rows.len() > visible {
        let lane = [
            x + width - logic::BAR - 1.0,
            y + logic::HEADER + logic::PAD,
            logic::BAR - 4.0,
            visible as f32 * logic::ROW - (logic::ROW - logic::BUTTON),
        ];
        host.fill(batch, lane, SCROLL_TRACK);
        let share = visible as f32 / rows.len() as f32;
        let thumb = (lane[3] * share).max(10.0);
        let pages = (rows.len() - visible) as f32;
        let at = if pages > 0.0 { first as f32 / pages } else { 0.0 };
        host.fill(
            batch,
            [lane[0], lane[1] + (lane[3] - thumb) * at, lane[2], thumb],
            SCROLL_THUMB,
        );
    }
    // The hint line inside the panel's foot.
    let hint = if rows.len() > visible {
        "wheel scrolls · click to switch · Esc closes"
    } else {
        "click to switch · Esc closes"
    };
    let hint_width = game.native_text_width(hint);
    host.text(
        batch,
        hint,
        [x + (width - hint_width) / 2.0, y + height - logic::FOOTER + 7.0],
        MUTED,
        true,
    );
    // Below the panel: what happened last, or a sign-in's code.
    let mut line = y + height + 6.0;
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
}

/// A line of text under the panel, centred with it.
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
    if state().map_open {
        *canceled = true;
        return;
    }
    let view = view();
    let rows = view.rows();
    let window = [screen.rect[0] + screen.rect[2], screen.rect[1] + screen.rect[3]];
    let visible = logic::visible_rows(window).min(rows.len().max(1));
    let (first, count) = logic::window_of(view.scroll, rows.len(), visible);
    let panel = logic::panel(window, visible.max(1));
    let Some(index) = logic::hit(panel, count, screen.mouse) else {
        return;
    };
    let Some((action, label)) = rows.get(first + index).cloned() else {
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

/// The wheel over the screen (`native_screen_mouse_scrolled_pre`, API 93):
/// the list scrolls when the accounts don't all fit.
pub(super) fn wheel(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) {
    if game.native_mod_screen() != Some(ID) {
        return;
    }
    let steps = scroll[1].round() as i32;
    if steps == 0 {
        return;
    }
    if state().map_open {
        {
            let mut state = state();
            for _ in 0..steps.abs() {
                state.map_zoom = if steps > 0 {
                    logic::zoom_out(state.map_zoom)
                } else {
                    logic::zoom_in(state.map_zoom)
                };
            }
        }
        *canceled = true;
        return;
    }
    let rows = view().rows().len();
    let window = [screen.rect[0] + screen.rect[2], screen.rect[1] + screen.rect[3]];
    let visible = logic::visible_rows(window);
    if rows <= visible {
        return;
    }
    {
        let mut state = state();
        state.scroll = logic::scrolled(state.scroll, rows, visible, steps);
    }
    *canceled = true;
}



/// What one world's map holds: a colour and a height per block, in tiles.
type Tiles = BTreeMap<(i32, i32), Box<[u32; TILE_CELLS]>>;

impl State {
    /// Switches the map to the world the player is in and parks the old one.
    /// True when the world changed, so the caller samples at once.
    fn activate(&mut self, key: String) -> bool {
        if self.active == key {
            return false;
        }
        let old = std::mem::replace(&mut self.active, key.clone());
        if !old.is_empty() {
            let tiles = std::mem::take(&mut self.tiles);
            let order = std::mem::take(&mut self.tile_order);
            self.parked.insert(old, (tiles, order));
        }
        if let Some((tiles, order)) = self.parked.remove(&key) {
            self.tiles = tiles;
            self.tile_order = order;
        }
        while self.parked.len() > 6 {
            let Some(oldest) = self.parked.keys().next().cloned() else {
                break;
            };
            self.parked.remove(&oldest);
        }
        self.explored_at = None;
        true
    }
}

/// What world the player is in: the server they joined and the dimension, the
/// two things that make a map its own.
fn world_key(game: &crate::game::Game<'_>) -> String {
    let place = state()
        .last
        .as_ref()
        .map_or_else(|| "Singleplayer".to_string(), |(address, _)| address.clone());
    let dimension = match game.dim {
        crate::game::Dim::Nether => "Nether",
        crate::game::Dim::End => "End",
        _ => "Overworld",
    };
    format!("{place} · {dimension}")
}

/// How many colours one stored tile holds.
const TILE_CELLS: usize = (logic::TILE * logic::TILE) as usize;

/// Remembers one block's surface colour, keeping the map inside its cap by
/// dropping the tiles looked at longest ago.
fn remember(state: &mut State, x: i32, z: i32, colour: u32) {
    if colour == 0 {
        return;
    }
    let key = logic::tile_key(x, z);
    if !state.tiles.contains_key(&key) {
        state.tiles.insert(key, Box::new([0u32; TILE_CELLS]));
        state.tile_order.push_back(key);
        while state.tile_order.len() > logic::TILE_CAP {
            if let Some(oldest) = state.tile_order.pop_front() {
                state.tiles.remove(&oldest);
            }
        }
    }
    if let Some(tile) = state.tiles.get_mut(&key) {
        let slot = logic::tile_slot(x, z);
        if let Some(cell) = tile.get_mut(slot) {
            *cell = colour;
        }
    }
}

/// What the map already knows about a block: nought when it has never been
/// seen, so nothing is drawn there.
fn seen(state: &State, x: i32, z: i32) -> u32 {
    let Some(tile) = state.tiles.get(&logic::tile_key(x, z)) else {
        return 0;
    };
    tile.get(logic::tile_slot(x, z)).copied().unwrap_or(0)
}

/// One map cell from a block's vanilla map colour id and its height: the colour
/// and the height packed together, so the map can shade it by the slope later.
fn map_pixel(colour: u8, height: i32) -> u32 {
    let rgb = crate::blocks::MAP_COLORS
        .get(colour as usize)
        .copied()
        .unwrap_or(0);
    if colour == 0 || rgb == 0 {
        // Seen, but the block has no map colour: a dark grey, not a hole.
        return logic::pack_cell(0x18_18_1C, height);
    }
    logic::pack_cell(rgb, height)
}

/// The player's column, to the block.
fn player_center(game: &crate::game::Game<'_>) -> Option<(i32, i32)> {
    let position = game.player.as_ref()?.position;
    Some((position.x.floor() as i32, position.z.floor() as i32))
}

/// The world map's cells: 16 by 9, as the window usually is.
fn big_cells() -> (u32, u32) {
    (logic::BIG, logic::BIG * 9 / 16)
}

/// Where the world map sits in `window`: 16 by 9, centred, under the top bar
/// and above the bottom one, so its cells stay square.
fn map_area(window: [f32; 2]) -> [f32; 4] {
    let room = (window[1] - 34.0).max(32.0);
    let width = (room * 16.0 / 9.0).min(window[0]);
    let height = (width * 9.0 / 16.0).min(room);
    [
        (window[0] - width) / 2.0,
        16.0 + (room - height) / 2.0,
        width,
        height,
    ]
}

/// Looks at every block in reach and stores its surface colour in the map, so
/// the terrain stays on the map after its chunk unloads. Runs at most every
/// `mini_every` milliseconds, and only after the player moved a block.
pub(super) fn explore(game: &mut crate::game::Game<'_>) {
    let Some(center) = player_center(game) else {
        return;
    };
    {
        // A new server or dimension gets its own map.
        let key = world_key(game);
        let changed = state().activate(key);
        let state = state();
        if !changed && let Some((built, at)) = state.explored_at {
            if built == center || at.elapsed().as_millis() < state.mini_every {
                return;
            }
        }
    }
    let started = std::time::Instant::now();
    let half = (logic::MINI / 2) as i32;
    let mut loaded = 0usize;
    let mut debug = String::from("centre: not loaded");
    {
        let world = &game.world;
        let facts = game.res.facts();
        let is_air = |block| facts.is_air(block);
        let mut state = state();
        for dz in -half..half {
            for dx in -half..half {
                let (x, z) = (center.0 + dx, center.1 + dz);
                if !world.loaded(x, z) {
                    continue;
                }
                loaded += 1;
                let surface = world.world_surface(x, z, is_air);
                let block = world.block(x, surface - 1, z);
                let entry = facts.states.get(block.0 as usize);
                let colour = entry.map_or(0, |state| state.map_color);
                remember(&mut state, x, z, map_pixel(colour, surface));
                if dx == 0 && dz == 0 {
                    debug = format!(
                        "centre {x},{z} y{}: {} colour {colour} surface {surface}",
                        surface - 1,
                        entry.map_or("?", |state| &state.name)
                    );
                }
            }
        }
        // 24 passes a second while one is cheap, fewer when it is not.
        let micros = started.elapsed().as_micros();
        let every = if micros > 6_000 {
            83
        } else if micros > 3_000 {
            55
        } else {
            42
        };
        let tiles: usize = state.tiles.len();
        state.mini_every = every;
        state.explored_at = Some((center, std::time::Instant::now()));
        state.map_debug = format!(
            "{loaded}/{} loaded · {tiles} tiles seen · {every} ms · {micros} µs · {debug}",
            (logic::MINI * logic::MINI) as usize
        );
        let first = !state.map_said;
        state.map_said = true;
        drop(state);
        if first {
            tell(format!("Account Switch map: {loaded} blocks in reach seen"));
        }
    }
}

/// The map's cells from what has been seen: `cols` by `rows` around `centre`,
/// one cell every `zoom` blocks, `offset` in pixels for the part of a block
/// the player stands past the last whole one, cut to a disc when `round`.
fn draw_cells(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    area: [f32; 4],
    state: &State,
    centre: (i32, i32),
    offset: [f32; 2],
    zoom: u32,
    cols: u32,
    rows: u32,
    round: bool,
) {
    let [x, y, width, height] = area;
    let cell_width = width / cols as f32;
    let cell_height = height / rows as f32;
    let half_column = (cols / 2) as i32;
    let half_row = (rows / 2) as i32;
    let step = zoom.max(1) as i32;
    for row in 0..rows {
        let top = y + row as f32 * cell_height + offset[1];
        let z = centre.1 + (row as i32 - half_row) * step;
        for column in 0..cols {
            if round && !logic::in_circle(column, row, cols) {
                continue;
            }
            let (dx, dz) = (
                centre.0 + (column as i32 - half_column) * step,
                z,
            );
            let packed = seen(state, dx, dz);
            if packed == 0 {
                continue;
            }
            // Shaded by the slope to the north and west, as a relief map is:
            // that is what makes it read as land instead of flat squares.
            let north = seen(state, dx, dz - step);
            let west = seen(state, dx - step, dz);
            let colour = logic::relief(
                logic::cell_rgb(packed),
                logic::cell_height(packed),
                (north != 0).then(|| logic::cell_height(north)),
                (west != 0).then(|| logic::cell_height(west)),
                logic::jitter(dx, dz),
            );
            let left = x + column as f32 * cell_width + offset[0];
            host.fill(
                batch,
                [left, top, cell_width + 0.5, cell_height + 0.5],
                colour,
            );
        }
    }
}

/// The player's arrow, as Xaero's minimap has it: a triangle turned to where
/// the player looks, with a dark shadow under it.
fn draw_arrow(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    center: [f32; 2],
    yaw: f32,
    pixel: f32,
) {
    let cells = 11u32;
    let half = cells as f32 / 2.0;
    for (shadow, colour) in [(1.0, 0xFF_10_10_14), (0.0, 0xFF_FF_FF_FF)] {
        for (column, row) in logic::arrow(cells, yaw) {
            host.fill(
                batch,
                [
                    center[0] + (column as f32 - half) * pixel + shadow,
                    center[1] + (row as f32 - half) * pixel + shadow,
                    pixel,
                    pixel,
                ],
                colour,
            );
        }
    }
}

/// A waypoint: a diamond with a dark edge, as the World Map draws them.
fn draw_waypoint(
    host: &dyn crate::mod_api::Host,
    batch: &mut crate::hud::HudBatch,
    at: [f32; 2],
    size: f32,
) {
    host.fill(batch, [at[0] - size / 2.0 - 1.0, at[1] - 1.0, size + 2.0, 3.0], 0xFF_10_10_14);
    host.fill(batch, [at[0] - 1.0, at[1] - size / 2.0 - 1.0, 3.0, size + 2.0], 0xFF_10_10_14);
    host.fill(batch, [at[0] - size / 2.0, at[1] - 1.0, size, 3.0], 0xFF_40_E0_E0);
    host.fill(batch, [at[0] - 1.0, at[1] - size / 2.0, 3.0, size], 0xFF_40_E0_E0);
}

/// The round minimap in its corner, over the hotbar's HUD layer
/// (`native_gui_layer_post`, API 139), as Xaero's Minimap has it.
pub(super) fn hud_post(
    game: &crate::game::Game<'_>,
    layer: &str,
    batch: &mut crate::hud::HudBatch,
) {
    // The game names its layers with the namespace: `minecraft:hotbar`.
    if !(layer == "minecraft:hotbar" || layer == "hotbar") || !game.native_playing() {
        return;
    }
    let state = state();
    if state.tiles.is_empty() {
        return;
    }
    let Some(centre) = player_center(game) else {
        return;
    };
    let host: &dyn crate::mod_api::Host = game;
    let side = logic::MINIMAP;
    let at = logic::minimap_at([batch.width, batch.height], state.corner, side);
    // A dark disc under the map, one fill per row, so the round edge does not
    // show the world and blocks never seen have a background.
    let radius = side / 2.0;
    for row in 0..side as u32 {
        let dy = row as f32 + 0.5 - radius;
        let half = (radius * radius - dy * dy).max(0.0).sqrt();
        if half <= 0.0 {
            continue;
        }
        host.fill(
            batch,
            [at[0] + radius - half, at[1] + row as f32, half * 2.0, 1.0],
            0xE0_08_08_0C,
        );
    }
    // The part of a block the player stands past the last whole one: the map
    // glides by it instead of stepping a whole cell.
    let zoom = state.mini_zoom.max(1);
    let cell = side / logic::MINI as f32;
    let offset = game
        .player
        .as_ref()
        .map(|player| {
            [
                -(player.position.x - centre.0 as f64) as f32 * cell / zoom as f32,
                -(player.position.z - centre.1 as f64) as f32 * cell / zoom as f32,
            ]
        })
        .unwrap_or([0.0, 0.0]);
    draw_cells(
        host,
        batch,
        [at[0], at[1], side, side],
        &state,
        centre,
        offset,
        zoom,
        logic::MINI,
        logic::MINI,
        true,
    );
    // The frame: dark outside, light inside, as the minimap's ring.
    for square in logic::ring(side + 4.0, 3.0, 96) {
        host.fill(
            batch,
            [at[0] - 2.0 + square[0], at[1] - 2.0 + square[1], square[2], square[3]],
            0xFF_1A_1A_20,
        );
    }
    for square in logic::ring(side, 2.0, 96) {
        host.fill(
            batch,
            [at[0] + square[0], at[1] + square[1], square[2], square[3]],
            0xFF_C8_C8_D4,
        );
    }
    for (letter, position) in logic::compass(side) {
        let width = game.native_text_width(letter);
        host.text(
            batch,
            letter,
            [at[0] + position[0] - width / 2.0 + 2.0, at[1] + position[1]],
            0xFF_FF_FF_FF,
            true,
        );
    }
    if let Some(player) = game.player.as_ref() {
        draw_arrow(
            host,
            batch,
            [at[0] + side / 2.0, at[1] + side / 2.0],
            player.yaw,
            1.0,
        );
        let line = logic::coords(
            player.position.x.floor() as i32,
            player.position.y.floor() as i32,
            player.position.z.floor() as i32,
        );
        let width = game.native_text_width(&line);
        let left = at[0] + side / 2.0 - width / 2.0 - 3.0;
        let top = at[1] + side + 7.0;
        host.fill(batch, [left, top, width + 6.0, 12.0], 0xC0_08_08_0C);
        host.text(batch, &line, [left + 3.0, top + 2.0], 0xFF_FF_FF_FF, true);
    }
}

/// Dragging on the mod's own screen pans the world map by the mouse's own
/// movement, in blocks of the map's own zoom (`native_screen_mouse_dragged_pre`).
pub(super) fn dragged(
    _game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    delta: [f32; 2],
    canceled: &mut bool,
) {
    if button != 0 {
        return;
    }
    let window = [
        screen.rect[0] + screen.rect[2],
        screen.rect[1] + screen.rect[3],
    ];
    let mut state = state();
    if !state.map_open {
        return;
    }
    let zoom = state.map_zoom.max(1) as f32;
    let area = map_area(window);
    let (cols, _) = big_cells();
    let cell = area[2] / cols as f32;
    // Drag right: the map follows the hand, so the view moves left.
    state.pan.0 -= (delta[0] * zoom / cell).round() as i32;
    state.pan.1 -= (delta[1] * zoom / cell).round() as i32;
    *canceled = true;
}

/// The world map over the world, as Xaero's World Map has it: the whole window
/// full of everything seen so far, coordinates on top, waypoints and a scale.
fn draw_map(
    game: &crate::game::Game<'_>,
    batch: &mut crate::hud::HudBatch,
    screen: &crate::mod_api::NativeScreenView<'_>,
) {
    let host: &dyn crate::mod_api::Host = game;
    let window = [
        screen.rect[0] + screen.rect[2],
        screen.rect[1] + screen.rect[3],
    ];
    host.fill(batch, [0.0, 0.0, window[0], window[1]], 0xFF_05_05_08);
    let area = map_area(window);
    let state = state();
    let zoom = state.map_zoom.max(1);
    let (cols, rows) = big_cells();
    let cell = area[2] / cols as f32;
    let player = player_center(game).unwrap_or((0, 0));
    let middle = (player.0 + state.pan.0, player.1 + state.pan.1);
    let offset = game
        .player
        .as_ref()
        .map(|me| {
            [
                -(me.position.x - player.0 as f64) as f32 * cell / zoom as f32,
                -(me.position.z - player.1 as f64) as f32 * cell / zoom as f32,
            ]
        })
        .unwrap_or([0.0, 0.0]);
    draw_cells(
        host, batch, area, &state, middle, offset, zoom, cols, rows, false,
    );
    // The player, where they really are on the map.
    let me = [
        area[0] + area[2] / 2.0 + offset[0] - state.pan.0 as f32 * cell / zoom as f32,
        area[1] + area[3] / 2.0 + offset[1] - state.pan.1 as f32 * cell / zoom as f32,
    ];
    for waypoint in &state.waypoints {
        let dx = (waypoint.0 - middle.0) as f32 * cell / zoom as f32;
        let dz = (waypoint.1 - middle.1) as f32 * cell / zoom as f32;
        let at = [area[0] + area[2] / 2.0 + dx, area[1] + area[3] / 2.0 + dz];
        if at[0] > area[0] && at[0] < area[0] + area[2] && at[1] > area[1] && at[1] < area[1] + area[3] {
            draw_waypoint(host, batch, at, 5.0);
        }
    }
    if me[0] > area[0] && me[0] < area[0] + area[2] && me[1] > area[1] && me[1] < area[1] + area[3] {
        let yaw = game.player.as_ref().map_or(0.0, |player| player.yaw);
        draw_arrow(host, batch, me, yaw, 1.0);
    }
    // Top: where the middle of the map is and how far a cell reaches.
    let top = format!(
        "{}   ·   X {}  Z {}   ·   {} blocks per cell   ·   {} waypoints   ·   {} tiles seen",
        state.active,
        middle.0,
        middle.1,
        zoom,
        state.waypoints.len(),
        state.tiles.len()
    );
    host.fill(batch, [0.0, 0.0, window[0], 15.0], 0xE0_08_08_0C);
    host.text(batch, &top, [6.0, 4.0], 0xFF_FF_FF_FF, true);
    // Bottom: the scale, and what the keys do.
    let hint = format!("drag pans · wheel zooms ({zoom}) · B waypoint · C corner · M closes");
    host.fill(batch, [0.0, window[1] - 17.0, window[0], 17.0], 0xE0_08_08_0C);
    host.text(batch, &hint, [6.0, window[1] - 13.0], MUTED, true);
    let scale_blocks = zoom * 32;
    let scale_px = 32.0 * cell;
    let bar_x = window[0] - scale_px - 46.0;
    host.fill(batch, [bar_x, window[1] - 10.0, scale_px, 2.0], 0xFF_FF_FF_FF);
    host.fill(batch, [bar_x, window[1] - 13.0, 1.0, 5.0], 0xFF_FF_FF_FF);
    host.fill(batch, [bar_x + scale_px, window[1] - 13.0, 1.0, 5.0], 0xFF_FF_FF_FF);
    host.text(
        batch,
        &format!("{scale_blocks}"),
        [bar_x + scale_px + 3.0, window[1] - 14.0],
        0xFF_FF_FF_FF,
        true,
    );
    let debug = state.map_debug.clone();
    if !debug.is_empty() {
        host.text(batch, &debug, [6.0, 18.0], CODE, true);
    }
}

pub(super) fn on_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !game.native_window() {
        return;
    }
    // The map key: the world map opens and closes.
    if key == game.native_mod_key(ID, MAP_KEY, MAP_DEFAULT) {
        if !pressed || !game.native_playing() {
            return;
        }
        let open = {
            let mut state = state();
            state.map_open = !state.map_open;
            state.map_open
        };
        if open {
            game.native_mod_screen_open(ID);
        } else if game.native_mod_screen() == Some(ID) {
            game.native_mod_screen_close(ID);
        }
        return;
    }
    // The minimap's zoom: one key out, one key in.
    if key == game.native_mod_key(ID, MINI_OUT_KEY, MINI_OUT_DEFAULT) {
        if pressed {
            let mut state = state();
            state.mini_zoom = logic::zoom_out(state.mini_zoom).clamp(1, 8);
        }
        return;
    }
    if key == game.native_mod_key(ID, MINI_IN_KEY, MINI_IN_DEFAULT) {
        if pressed {
            let mut state = state();
            state.mini_zoom = logic::zoom_in(state.mini_zoom).clamp(1, 8);
        }
        return;
    }
    // The waypoint key: a marker where the player stands, as the World Map has it.
    if key == game.native_mod_key(ID, WAYPOINT_KEY, WAYPOINT_DEFAULT) {
        if pressed && game.native_playing()
            && let Some(center) = player_center(game)
        {
            let mut state = state();
            if !state.waypoints.contains(&center) {
                state.waypoints.push(center);
                if state.waypoints.len() > 64 {
                    state.waypoints.remove(0);
                }
            }
            let count = state.waypoints.len();
            drop(state);
            tell(format!("Waypoint at {},{} ({count} set)", center.0, center.1));
        }
        return;
    }
    // The zoom key: held while playing, as OptiFine's zoom.
    if key == game.native_mod_key(ID, ZOOM_KEY, ZOOM_DEFAULT) {
        if pressed && !game.native_playing() {
            return;
        }
        state().zoom = pressed.then_some(logic::ZOOM_START);
        return;
    }
    if !pressed || key != game.native_mod_key(ID, KEY, DEFAULT_KEY) {
        return;
    }
    if game.native_mod_screen() == Some(ID) {
        game.native_mod_screen_close(ID);
        return;
    }
    open(game);
}

/// The zoom in effect, unless another mod's came first
/// (`native_zoom`'s hook).
pub(super) fn zoom_view(zoom: &mut Option<crate::game::native_hooks::NativeZoom>) {
    if zoom.is_some() {
        return;
    }
    let Some(factor) = state().zoom else {
        return;
    };
    *zoom = Some(crate::game::native_hooks::NativeZoom {
        factor,
        smooth: true,
        turn: logic::zoom_turn(factor),
    });
}

/// The wheel while the zoom key is held: it changes how far
/// (`native_wheel`'s hook). Claimed, so the hotbar stays put.
pub(super) fn zoom_wheel(steps: f32, claimed: &mut bool) {
    if *claimed {
        return;
    }
    let mut state = state();
    if let Some(factor) = state.zoom {
        state.zoom = Some(logic::zoomed(factor, steps));
        *claimed = true;
    }
}

/// The mod's row in Options > Mods (`native_mods`' hook).
pub(super) fn listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: ID.to_owned(),
        title: format!("Account Switch {VERSION}"),
        description: DESCRIPTION.to_owned(),
        keys: vec![
            [
                KEY.to_owned(),
                "Open the account switcher".to_owned(),
                DEFAULT_KEY.to_owned(),
            ],
            [
                ZOOM_KEY.to_owned(),
                "Zoom (hold)".to_owned(),
                ZOOM_DEFAULT.to_owned(),
            ],
            [
                MAP_KEY.to_owned(),
                "Minimap: big map".to_owned(),
                MAP_DEFAULT.to_owned(),
            ],
        ],
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
    // Below the accounts: which corner the minimap shows in.
    buttons.push(format!(
        "Minimap corner: {} (click to move)",
        logic::corner_name(state().corner)
    ));
}

/// A click on the page (`native_settings_click`'s hook).
pub(super) fn clicked_page(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    let view = view();
    let rows = view.rows();
    if button >= rows.len() {
        // The corner row: the next corner.
        let mut state = state();
        state.corner = logic::corner(state.corner + 1);
        let name = logic::corner_name(state.corner);
        drop(state);
        game.system_message(horizon_client::text::Text::literal(format!(
            "Account Switch: minimap corner {name}"
        )));
        return;
    }
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
