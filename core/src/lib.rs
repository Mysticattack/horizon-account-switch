//! The logic of Account Switch, apart from the game: what the Horizon launcher
//! prints about its accounts, which account comes next, and what the settings
//! page shows. Everything here is testable without the game.
//!
//! The game-side module only runs the launcher, keeps this state and calls the
//! game's own leave/join functions.

/// A signed-in Microsoft account, as `horizon-launcher account list --json`
/// gives it (`launcher/src/cli.rs`, `AccountCmd::List`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub uuid: String,
    /// The launcher's pick for the next start (`accounts.json`).
    pub selected: bool,
    /// `ready`, `needs_refresh`, `sign_in_required`, `retry` or
    /// `ownership_required`; empty when the answer carries no status.
    pub state: String,
    /// The launcher's own sentence for that state, empty when ready.
    pub message: String,
}

/// The answer of `account list --json`: its accounts, in the launcher's order.
pub fn parse_list(text: &str) -> Result<Vec<Account>, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("the launcher's answer is not JSON: {error}"))?;
    let Some(list) = value.get("accounts").and_then(|list| list.as_array()) else {
        return Err("the launcher's answer has no \"accounts\" list".to_owned());
    };
    let mut accounts = Vec::with_capacity(list.len());
    for entry in list {
        let name = entry
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_owned();
        if name.is_empty() {
            continue;
        }
        let text_of = |object: &serde_json::Value, key: &str| {
            object
                .get(key)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let authentication = entry.get("authentication");
        accounts.push(Account {
            uuid: text_of(entry, "uuid"),
            selected: entry
                .get("selected")
                .and_then(|selected| selected.as_bool())
                .unwrap_or(false),
            state: authentication.map_or_else(String::new, |status| text_of(status, "state")),
            message: authentication.map_or_else(String::new, |status| text_of(status, "message")),
            name,
        });
    }
    Ok(accounts)
}

/// The session file from `account session <WHO>`: the bare path it prints, or
/// the `{"session_file": …}` of `--json`. The error is the launcher's own
/// sentence (`{"authentication": {"message": …}}`, or what it printed).
pub fn parse_session_file(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the launcher printed nothing".to_owned());
    }
    if !text.starts_with('{') {
        // Without --json: one path per line.
        return text
            .lines()
            .last()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .ok_or_else(|| "the launcher printed no session file".to_owned());
    }
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("the launcher's answer is not JSON: {error}"))?;
    if let Some(path) = value.get("session_file").and_then(|path| path.as_str()) {
        return Ok(path.to_owned());
    }
    if let Some(message) = value
        .get("authentication")
        .and_then(|status| status.get("message"))
        .and_then(|message| message.as_str())
    {
        return Err(message.to_owned());
    }
    Err("the launcher's answer has no session file".to_owned())
}

/// The account after `playing` in the launcher's order, wrapping round: what
/// the mod's key switches to. None when no account is signed in. An account
/// the list doesn't know (a session file of an account since signed out)
/// starts the cycle at the first one.
pub fn next_account(accounts: &[Account], playing: Option<&str>) -> Option<usize> {
    if accounts.is_empty() {
        return None;
    }
    let at = playing.and_then(|name| {
        accounts
            .iter()
            .position(|account| account.name.eq_ignore_ascii_case(name))
    });
    Some(match at {
        Some(at) => (at + 1) % accounts.len(),
        None => 0,
    })
}

/// The buttons of the mod's settings page, in the order the clicks come back:
/// `Refresh accounts` first, then one row per account (button `i + 1`).
pub fn labels(accounts: &[Account], playing: Option<&str>, listing: bool) -> Vec<String> {
    let mut labels = vec![if listing {
        "Loading accounts…".to_owned()
    } else {
        "Refresh accounts".to_owned()
    }];
    for account in accounts {
        let here = playing.is_some_and(|name| account.name.eq_ignore_ascii_case(name));
        labels.push(if here {
            format!("{} — playing now", account.name)
        } else {
            format!("Play as {}", account.name)
        });
    }
    if accounts.is_empty() && !listing {
        labels.push("No signed-in account found — sign in in the launcher".to_owned());
    }
    labels
}

/// What a click on `labels`' button `button` means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// Button 0: ask the launcher again.
    Refresh,
    /// A row after it: switch to the account at this index.
    Account(usize),
    /// Not a button of this page, or the "no account" row.
    Nothing,
}

pub fn click(labels: usize, accounts: usize, button: usize) -> Click {
    if button == 0 {
        return Click::Refresh;
    }
    if button <= accounts && button < labels {
        return Click::Account(button - 1);
    }
    Click::Nothing
}

/// The link and the code of the sign-in line the launcher prints on stderr:
/// `To sign in, open https://www.microsoft.com/link in a browser and enter the
/// code ABC-DEF`. None for every other line.
pub fn parse_sign_in(line: &str) -> Option<(String, String)> {
    let (before, code) = line.split_once(" and enter the code ")?;
    let (_, rest) = before.rsplit_once("open ")?;
    // The link ends at the first space (`… in a browser` follows it).
    let link = rest.split_whitespace().next()?;
    let code = code.trim();
    (!link.is_empty() && !code.is_empty()).then(|| (link.to_owned(), code.to_owned()))
}

/// The account the launcher says it signed in: `Signed in as Alex.` (or
/// `Signed in as Alex again.`). None for every other line.
pub fn signed_in_name(text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let rest = line.trim().strip_prefix("Signed in as ")?;
        let rest = rest.strip_suffix(" again.").unwrap_or(rest);
        let name = rest.strip_suffix('.').unwrap_or(rest).trim();
        (!name.is_empty()).then(|| name.to_owned())
    })
}

/// The distance from one row's top to the next.
pub const ROW: f32 = 24.0;
/// A row's drawn height.
pub const BUTTON: f32 = 20.0;
/// The panel's width; names of six or sixteen letters fit either side.
pub const WIDTH: f32 = 260.0;
/// The title bar above the rows.
pub const HEADER: f32 = 28.0;
/// The hint line below the rows, inside the panel.
pub const FOOTER: f32 = 22.0;
/// Room between the panel's edge and its rows.
pub const PAD: f32 = 8.0;
/// The lane on the right the scrollbar uses.
pub const BAR: f32 = 8.0;
/// The margin the panel keeps from the window's edge.
pub const MARGIN: f32 = 12.0;

/// The map texture's side in texels: the game's minimap slot is 128 square, so
/// `native_mod_image` is not needed at all — the map goes into that slot.
pub const TEXELS: u32 = 128;
/// The world map's cells per side: four times the minimap's, so it is sampled
/// sharper instead of being the minimap drawn larger.
pub const BIG: u32 = 160;
/// The minimap's cells per side, at one block per cell.
pub const MINI: u32 = 96;
/// The corner minimap's side in GUI pixels, and the texels it shows of the map.
pub const MINIMAP: f32 = 96.0;
pub const MINIMAP_TEXELS: f32 = MINI as f32;
/// The closest and furthest a texel can be, in blocks.
pub const MAP_ZOOM_MIN: u32 = 1;
pub const MAP_ZOOM_MAX: u32 = 32;

/// The next zoom in or out: powers of two inside the limits.
pub fn zoom_in(zoom: u32) -> u32 {
    (zoom * 2).min(MAP_ZOOM_MAX)
}

pub fn zoom_out(zoom: u32) -> u32 {
    (zoom / 2).max(MAP_ZOOM_MIN)
}

/// How many blocks one side of the map image covers.
pub fn map_blocks(zoom: u32) -> i32 {
    TEXELS as i32 * zoom.max(1) as i32
}

/// The part of the map texture the corner minimap shows: `MINIMAP_TEXELS`
/// square, centred.
pub fn minimap_crop() -> [f32; 4] {
    let start = (TEXELS as f32 - MINIMAP_TEXELS) / 2.0;
    [start, start, MINIMAP_TEXELS, MINIMAP_TEXELS]
}

/// Where the corner minimap sits in a window: the game keeps its extras two
/// pixels from the edge.
pub fn minimap_at(window: [f32; 2], index: usize, side: f32) -> [f32; 2] {
    const EDGE: f32 = 2.0;
    let left = matches!(corner(index), 0 | 2);
    let top = matches!(corner(index), 0 | 1);
    let x = if left {
        EDGE
    } else {
        (window[0] - EDGE - side).max(0.0)
    };
    let y = if top {
        EDGE
    } else {
        (window[1] - EDGE - side).max(0.0)
    };
    [x, y]
}

/// The four corners, in the order the settings row cycles them.
pub fn corner(index: usize) -> usize {
    index % 4
}

pub fn corner_name(index: usize) -> &'static str {
    match corner(index) {
        0 => "top left",
        1 => "top right",
        2 => "bottom left",
        _ => "bottom right",
    }
}

/// The zoom the key starts at, how far the wheel takes it, and its limits.
pub const ZOOM_START: f32 = 4.0;
pub const ZOOM_MIN: f32 = 1.25;
pub const ZOOM_MAX: f32 = 10.0;
pub const ZOOM_STEP: f32 = 1.25;

/// The zoom after `steps` wheel steps while the key is held.
pub fn zoomed(factor: f32, steps: f32) -> f32 {
    (factor * ZOOM_STEP.powf(steps)).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// How much slower the view turns while zoomed in, as the spyglass does.
pub fn zoom_turn(factor: f32) -> f32 {
    1.0 / factor.max(1.0)
}

/// How many rows the window can show at once: as many as fit between the
/// margins, with the title bar and the hint line.
pub fn visible_rows(window: [f32; 2]) -> usize {
    let room = window[1] - MARGIN * 2.0 - HEADER - PAD * 2.0 - FOOTER;
    let fit = ((room + (ROW - BUTTON)) / ROW).floor();
    (fit as i32).max(1) as usize
}

/// The panel in GUI pixels (`[x, y, width, height]`), centred in the window,
/// holding `visible` rows.
pub fn panel(window: [f32; 2], visible: usize) -> [f32; 4] {
    let width = WIDTH.min(window[0] - MARGIN * 2.0).max(180.0);
    let rows = visible as f32 * ROW - (ROW - BUTTON);
    let height = HEADER + PAD + rows + PAD + FOOTER;
    let x = ((window[0] - width) / 2.0).floor().max(0.0);
    let y = ((window[1] - height) / 2.0).floor().max(MARGIN);
    [x, y, width, height]
}

/// The rectangle of visible row `index` (the first one shown, not the first
/// of the list).
pub fn row(panel: [f32; 4], index: usize) -> [f32; 4] {
    [
        panel[0] + PAD,
        panel[1] + HEADER + PAD + index as f32 * ROW,
        panel[2] - PAD * 2.0 - BAR,
        BUTTON,
    ]
}

/// The visible row under `mouse`, if any.
pub fn hit(panel: [f32; 4], visible: usize, mouse: [f32; 2]) -> Option<usize> {
    (0..visible).find(|index| {
        let [x, y, width, height] = row(panel, *index);
        mouse[0] >= x && mouse[0] < x + width && mouse[1] >= y && mouse[1] < y + height
    })
}

/// Where the list starts after `steps` wheel steps (down the list is
/// positive, as the game reports the wheel), kept so the last page stays full.
pub fn scrolled(offset: usize, total: usize, visible: usize, steps: i32) -> usize {
    let last = total.saturating_sub(visible);
    (offset as i32 + steps).clamp(0, last as i32) as usize
}

/// The visible window of a list: `(first shown, how many)`.
pub fn window_of(offset: usize, total: usize, visible: usize) -> (usize, usize) {
    let first = offset.min(total.saturating_sub(visible));
    (first, total.saturating_sub(first).min(visible))
}

/// The world lattice a map samples on: cells sit on multiples of `step`, so a
/// grid rebuilt around a new centre covers exactly the same cell boundaries and
/// the picture slides instead of shimmering.
pub fn aligned(center: i32, step: i32) -> i32 {
    let step = step.max(1);
    center.div_euclid(step) * step
}

/// Whether the cell at `column`,`row` of a `cells` square grid lies inside the
/// inscribed circle: the minimap is round, as Xaero's is.
pub fn in_circle(column: u32, row: u32, cells: u32) -> bool {
    let half = cells as f32 / 2.0;
    let dx = column as f32 + 0.5 - half;
    let dy = row as f32 + 0.5 - half;
    let radius = half - 0.5;
    dx * dx + dy * dy <= radius * radius
}

/// The frame's ring: `segments` small squares on the circle of `diameter`,
/// `thickness` wide, relative to the map's top left corner.
pub fn ring(diameter: f32, thickness: f32, segments: u32) -> Vec<[f32; 4]> {
    let centre = diameter / 2.0;
    let radius = centre - thickness / 2.0;
    (0..segments.max(1))
        .map(|step| {
            let angle = step as f32 / segments.max(1) as f32 * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            [
                centre + cos * radius - thickness / 2.0,
                centre + sin * radius - thickness / 2.0,
                thickness,
                thickness,
            ]
        })
        .collect()
}

/// The player's arrow: a triangle pointing along the way the player faces, as
/// the cells of a `cells` square grid that it covers. North is up, so a yaw of
/// nought (south) points down.
pub fn arrow(cells: u32, yaw: f32) -> Vec<(u32, u32)> {
    let half = cells as f32 / 2.0;
    let turn = yaw.to_radians();
    let (dx, dy) = (-turn.sin(), turn.cos());
    let tip = half - 1.0;
    let mut pixels = Vec::new();
    for row in 0..cells {
        for column in 0..cells {
            let x = column as f32 + 0.5 - half;
            let y = row as f32 + 0.5 - half;
            let along = x * dx + y * dy;
            let across = -x * dy + y * dx;
            if along >= -tip * 0.4 && along <= tip && across.abs() <= (tip - along) * 0.7 + 0.4 {
                pixels.push((column, row));
            }
        }
    }
    pixels
}

/// The compass letters around the minimap: north up, then east, south, west.
pub fn compass(diameter: f32) -> [(&'static str, [f32; 2]); 4] {
    let edge = diameter + 4.0;
    [
        ("N", [diameter / 2.0 - 2.0, -8.0]),
        ("E", [edge - 2.0, diameter / 2.0 - 4.0]),
        ("S", [diameter / 2.0 - 2.0, edge]),
        ("W", [-4.0, diameter / 2.0 - 4.0]),
    ]
}

/// The coordinates line the Minimap shows under its circle.
pub fn coords(x: i32, y: i32, z: i32) -> String {
    format!("{x} {y} {z}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
      "accounts": [
        {"name": "Redstonerthebest", "uuid": "069a79f444e94726a5befca90e38aaf5", "selected": true,
         "authentication": {"state": "ready", "expires_at": 1793600000, "message": "Ready"}},
        {"name": "AltAccount", "uuid": "11111111222233334444555555555555", "selected": false,
         "authentication": {"state": "sign_in_required", "expires_at": null,
                            "message": "Sign in again to play as AltAccount"}}
      ],
      "offline": [{"name": "Player", "selected": false}],
      "sign_in_configured": true,
      "owner": true
    }"#;

    #[test]
    fn list_keeps_the_launchers_order_and_its_status() {
        let accounts = parse_list(LIST).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].name, "Redstonerthebest");
        assert!(accounts[0].selected);
        assert_eq!(accounts[0].state, "ready");
        assert_eq!(accounts[1].name, "AltAccount");
        assert!(!accounts[1].selected);
        assert_eq!(accounts[1].state, "sign_in_required");
        assert_eq!(accounts[1].message, "Sign in again to play as AltAccount");
    }

    #[test]
    fn list_without_accounts_is_empty_not_an_error() {
        assert_eq!(
            parse_list(r#"{"accounts": [], "offline": []}"#).unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn list_without_the_key_is_an_error() {
        assert!(parse_list(r#"{"offline": []}"#).is_err());
        assert!(parse_list("No Microsoft account signed in").is_err());
    }

    #[test]
    fn session_file_of_json_and_plain_answers() {
        assert_eq!(
            parse_session_file(
                r#"{"session_file": "/home/p/.local/share/horizon/sessions/session-abc.json"}"#
            )
            .unwrap(),
            "/home/p/.local/share/horizon/sessions/session-abc.json"
        );
        assert_eq!(
            parse_session_file("/home/p/.local/share/horizon/sessions/session-abc.json\n").unwrap(),
            "/home/p/.local/share/horizon/sessions/session-abc.json"
        );
    }

    #[test]
    fn session_refusal_carries_the_launchers_sentence() {
        let refused = r#"{"authentication": {"state": "sign_in_required", "expires_at": null,
                          "message": "Sign in again to play as AltAccount"}}"#;
        assert_eq!(
            parse_session_file(refused).unwrap_err(),
            "Sign in again to play as AltAccount"
        );
        assert!(parse_session_file("   ").is_err());
        assert!(parse_session_file(r#"{"accounts": []}"#).is_err());
    }

    fn named(name: &str) -> Account {
        Account {
            name: name.to_owned(),
            uuid: String::new(),
            selected: false,
            state: String::new(),
            message: String::new(),
        }
    }

    #[test]
    fn the_key_cycles_through_the_launchers_order() {
        let accounts = vec![named("A"), named("B"), named("C")];
        assert_eq!(next_account(&accounts, Some("A")), Some(1));
        assert_eq!(next_account(&accounts, Some("c")), Some(0));
        assert_eq!(next_account(&accounts, Some("gone")), Some(0));
        assert_eq!(next_account(&accounts, None), Some(0));
        assert_eq!(next_account(&[], Some("A")), None);
    }

    #[test]
    fn labels_name_the_playing_account_and_the_refresh_row() {
        let accounts = parse_list(LIST).unwrap();
        let rows = labels(&accounts, Some("Redstonerthebest"), false);
        assert_eq!(
            rows,
            vec![
                "Refresh accounts",
                "Redstonerthebest — playing now",
                "Play as AltAccount",
            ]
        );
        let rows = labels(&accounts, None, true);
        assert_eq!(rows[0], "Loading accounts…");
        assert_eq!(rows[2], "Play as AltAccount");
        let empty = labels(&[], None, false);
        assert_eq!(empty.len(), 2);
        assert!(empty[1].starts_with("No signed-in account"));
    }

    #[test]
    fn clicks_map_to_the_buttons_that_are_there() {
        assert_eq!(click(3, 2, 0), Click::Refresh);
        assert_eq!(click(3, 2, 1), Click::Account(0));
        assert_eq!(click(3, 2, 2), Click::Account(1));
        assert_eq!(click(3, 2, 3), Click::Nothing);
        assert_eq!(click(2, 0, 1), Click::Nothing);
        assert_eq!(click(2, 0, 7), Click::Nothing);
    }

    #[test]
    fn the_launchers_signed_in_line_names_the_account() {
        assert_eq!(
            signed_in_name("Signed in as Alex."),
            Some("Alex".to_owned())
        );
        assert_eq!(
            signed_in_name("Signed in as Alex again."),
            Some("Alex".to_owned())
        );
        assert_eq!(
            signed_in_name("noise\nSigned in as Redstonerthebest.\n"),
            Some("Redstonerthebest".to_owned())
        );
        assert_eq!(signed_in_name("To sign in, open x"), None);
        assert_eq!(signed_in_name("Signed in as ."), None);
    }

    #[test]
    fn the_sign_in_line_gives_the_link_and_the_code() {
        let line = "To sign in, open https://www.microsoft.com/link in a browser and enter the code K7Q4-2XP9M";
        assert_eq!(
            parse_sign_in(line),
            Some((
                "https://www.microsoft.com/link".to_owned(),
                "K7Q4-2XP9M".to_owned()
            ))
        );
        let again = "To sign Alex in again, open https://www.microsoft.com/link in a browser and enter the code QQ12";
        assert_eq!(
            parse_sign_in(again).map(|(_, code)| code),
            Some("QQ12".to_owned())
        );
        assert_eq!(parse_sign_in("Signed in as Alex."), None);
        assert_eq!(parse_sign_in(""), None);
    }

    #[test]
    fn the_panel_holds_the_rows_it_shows_and_stays_in_the_window() {
        let window = [352.0, 203.0];
        let visible = visible_rows(window);
        assert!(visible >= 3);
        let p = panel(window, visible);
        assert_eq!(p[2], WIDTH);
        assert_eq!(p[0], ((352.0 - WIDTH) / 2.0).floor());
        // The last of the rows ends above the hint line.
        let last = row(p, visible - 1);
        assert!(last[1] + last[3] <= p[1] + p[3] - FOOTER);
        // Everything stays inside the window.
        assert!(p[1] >= MARGIN);
        assert!(p[1] + p[3] <= window[1] - MARGIN + 0.5);
        // A narrow window still gets a usable panel.
        assert!(panel([200.0, 203.0], 3)[2] >= 180.0);
    }

    #[test]
    fn the_map_zoom_moves_in_powers_of_two_and_stops_at_its_limits() {
        assert_eq!(zoom_in(1), 2);
        assert_eq!(zoom_in(32), MAP_ZOOM_MAX);
        assert_eq!(zoom_out(2), 1);
        assert_eq!(zoom_out(1), MAP_ZOOM_MIN);
        assert_eq!(map_blocks(1), 128);
        assert_eq!(map_blocks(32), 4096);
        // The corner map shows the middle 64 texels of the same image.
        assert_eq!(minimap_crop(), [16.0, 16.0, 96.0, 96.0]);
        assert_eq!(corner(4), 0);
        assert_eq!(corner_name(0), "top left");
        assert_eq!(corner_name(3), "bottom right");
    }

    #[test]
    fn the_minimap_sits_in_its_corner_two_pixels_from_the_edge() {
        let window = [352.0, 203.0];
        assert_eq!(minimap_at(window, 0, 72.0), [2.0, 2.0]);
        assert_eq!(minimap_at(window, 1, 72.0), [352.0 - 2.0 - 72.0, 2.0]);
        assert_eq!(minimap_at(window, 2, 72.0), [2.0, 203.0 - 2.0 - 72.0]);
        assert_eq!(
            minimap_at(window, 3, 72.0),
            [352.0 - 2.0 - 72.0, 203.0 - 2.0 - 72.0]
        );
    }

    #[test]
    fn the_wheel_zooms_between_the_limits() {
        assert_eq!(zoomed(ZOOM_START, 0.0), ZOOM_START);
        assert!(zoomed(ZOOM_START, 1.0) > ZOOM_START);
        assert!(zoomed(ZOOM_START, -1.0) < ZOOM_START);
        assert_eq!(zoomed(ZOOM_START, 40.0), ZOOM_MAX);
        assert_eq!(zoomed(ZOOM_START, -40.0), ZOOM_MIN);
        assert_eq!(zoom_turn(1.0), 1.0);
        assert_eq!(zoom_turn(4.0), 0.25);
    }

    #[test]
    fn many_accounts_scroll_and_keep_the_last_page_full() {
        // A tall window shows a dozen rows; twelve accounts still need
        // two pages of six.
        assert!(visible_rows([352.0, 400.0]) >= 12);
        let visible = 6;
        assert_eq!(scrolled(0, 12, visible, 1), 1);
        assert_eq!(scrolled(0, 12, visible, -1), 0);
        assert_eq!(scrolled(6, 12, visible, -5), 1);
        assert_eq!(scrolled(3, 12, visible, 5), 6);
        assert_eq!(scrolled(5, 4, visible, 1), 0);
        assert_eq!(window_of(6, 12, 6), (6, 6));
        assert_eq!(window_of(11, 12, 6), (6, 6));
        assert_eq!(window_of(0, 3, 6), (0, 3));
    }

    #[test]
    fn a_row_is_hit_only_where_it_is() {
        let window = [352.0, 400.0];
        let p = panel(window, 4);
        let [x, y, width, height] = row(p, 1);
        assert_eq!(hit(p, 4, [x + 1.0, y + 1.0]), Some(1));
        assert_eq!(hit(p, 4, [x + width - 1.0, y + height - 1.0]), Some(1));
        assert_eq!(hit(p, 4, [x + width + 1.0, y + 1.0]), None);
        assert_eq!(hit(p, 4, [x + 1.0, y + height + 1.0]), None);
        // The title bar above and the hint line below.
        assert_eq!(hit(p, 4, [x + 1.0, p[1] + 2.0]), None);
        assert_eq!(hit(p, 4, [x + 1.0, p[1] + p[3] - 4.0]), None);
        // Only the rows it was told about.
        let [_, below, _, _] = row(p, 4);
        assert_eq!(hit(p, 4, [x + 1.0, below + 1.0]), None);
    }

    #[test]
    fn cells_sit_on_the_world_lattice() {
        assert_eq!(aligned(21, 8), 16);
        assert_eq!(aligned(-3, 8), -8);
        assert_eq!(aligned(7, 1), 7);
        assert_eq!(aligned(0, 0), 0);
    }

    #[test]
    fn the_circle_leaves_the_corners_out() {
        assert!(in_circle(32, 32, 64));
        assert!(!in_circle(0, 0, 64));
        assert!(!in_circle(63, 63, 64));
        assert!(in_circle(32, 1, 64));
    }

    #[test]
    fn the_ring_has_one_square_per_segment() {
        let squares = ring(72.0, 3.0, 48);
        assert_eq!(squares.len(), 48);
        for square in &squares {
            assert!(square[0] > -2.0 && square[0] < 72.0);
            assert!(square[1] > -2.0 && square[1] < 72.0);
        }
    }

    #[test]
    fn the_arrow_points_where_the_player_faces() {
        let south = arrow(11, 0.0);
        assert!(!south.is_empty());
        assert!(south.iter().any(|(_, row)| *row >= 8));
        let north = arrow(11, 180.0);
        assert!(north.iter().any(|(_, row)| *row <= 2));
    }

    #[test]
    fn the_compass_is_north_east_south_west() {
        let letters = compass(72.0);
        assert_eq!(letters[0].0, "N");
        assert!(letters[0].1[1] < 0.0);
        assert_eq!(letters[2].0, "S");
        assert_eq!(coords(12, 64, -34), "12 64 -34");
    }
}
