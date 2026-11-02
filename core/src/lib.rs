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

/// The distance from one row's top to the next: a vanilla button's 20 pixels
/// and 4 between them.
pub const ROW: f32 = 24.0;
/// A row's drawn height: vanilla's button texture is 20 pixels tall.
pub const BUTTON: f32 = 20.0;
/// The button column's width: vanilla's buttons are 200 pixels wide.
pub const WIDTH: f32 = 200.0;
const MARGIN: f32 = 20.0;
/// The smallest room above the column: the title and the separator.
pub const TITLE_SPACE: f32 = 46.0;

/// The button column in GUI pixels (`[x, y, width, height]`), centred in the
/// window, `rows` buttons tall.
pub fn panel(window: [f32; 2], rows: usize) -> [f32; 4] {
    let width = WIDTH.min((window[0] - MARGIN).max(120.0));
    let height = (rows.max(1) as f32 * ROW - (ROW - BUTTON)).max(BUTTON);
    let x = ((window[0] - width) / 2.0).floor().max(0.0);
    let y = ((window[1] - height) / 2.0)
        .floor()
        .max(TITLE_SPACE)
        .min((window[1] - height).max(TITLE_SPACE));
    [x, y, width, height]
}

/// The rectangle of row `index` (`0` is the first button).
pub fn row(panel: [f32; 4], index: usize) -> [f32; 4] {
    [panel[0], panel[1] + index as f32 * ROW, panel[2], BUTTON]
}

/// The row under `mouse`, when `mouse` is inside the panel: `None` on the
/// title, the hint, the gaps or outside it.
pub fn hit(panel: [f32; 4], rows: usize, mouse: [f32; 2]) -> Option<usize> {
    (0..rows).find(|index| {
        let [x, y, width, height] = row(panel, *index);
        mouse[0] >= x && mouse[0] < x + width && mouse[1] >= y && mouse[1] < y + height
    })
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
    fn the_panel_is_centred_and_holds_its_rows() {
        let p = panel([320.0, 240.0], 3);
        assert_eq!(p[2], WIDTH);
        assert_eq!(p[0], (320.0 - WIDTH) / 2.0);
        // Three buttons: 20 + 4 + 20 + 4 + 20 = 68 tall, centred.
        assert_eq!(p[3], 3.0 * ROW - (ROW - BUTTON));
        assert_eq!(p[1], ((240.0 - p[3]) / 2.0).floor());
        let last = row(p, 2);
        assert_eq!(last[1] + last[3], p[1] + p[3]);
        // A small window keeps its margin rather than overflowing, and the
        // column never starts above the title's room.
        let small = panel([100.0, 80.0], 4);
        assert_eq!(small[2], 120.0);
        assert_eq!(small[0], 0.0);
        assert_eq!(small[1], TITLE_SPACE);
    }

    #[test]
    fn a_row_is_hit_only_where_it_is() {
        let p = panel([320.0, 240.0], 3);
        let [x, y, width, height] = row(p, 1);
        assert_eq!(hit(p, 3, [x + 1.0, y + 1.0]), Some(1));
        assert_eq!(hit(p, 3, [x + width - 1.0, y + height - 1.0]), Some(1));
        assert_eq!(hit(p, 3, [x + width, y + 1.0]), None);
        assert_eq!(hit(p, 3, [x + 1.0, y + height]), None);
        // Above the column (the title) and the gap between two rows.
        assert_eq!(hit(p, 3, [x + 1.0, p[1] - 4.0]), None);
        assert_eq!(hit(p, 3, [x + 1.0, y + height + 0.5]), None);
        // A row past the ones there are.
        let [_, below, _, _] = row(p, 3);
        assert_eq!(hit(p, 3, [x + 1.0, below + 1.0]), None);
    }
}
