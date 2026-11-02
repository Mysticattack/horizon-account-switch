# Account Switch — a native Horizon mod

Switch the Minecraft account you play as, without quitting the game.

## Kurz: installieren (Deutsch)

Das Repo ist das fertige Bundle. Horizon baut native Mods lokal, also klonen und bauen:

```sh
git clone https://github.com/Mysticattack/horizon-account-switch.git
horizon-launcher native-build horizon-account-switch --trust-native-code
```

Der erste Build lädt Rust und kompiliert das Spiel (20–30 Minuten), später geht es schneller.
Danach die Instanz starten, die er gebaut hat, und im Spiel **F9** drücken (Taste änderbar unter
Options → Mods → Account Switch): es öffnet sich ein Fenster mit einer Zeile pro eingeloggtem
Account — anklicken, fertig. Die Zeile **+ Add account** startet eine Microsoft-Anmeldung, **öffnet
die Anmeldeseite im Browser und legt den Code in die Zwischenablage** (Zeile `Copy code …` kopiert
ihn noch einmal), danach steht der neue Account in der Liste. **Esc** schließt.

Während ein Wechsel läuft, zeigt das Fenster nur noch `Switching to <Name>…` mit einem laufenden Balken und
nimmt keine weiteren Klicks an — so meldet man sich nicht versehentlich zweimal an. Klick auf den Account,
der schon spielt, oder ein Sign-in mit einem bereits angemeldeten Account sagt das ausdrücklich
(`<Name> is already signed in`).

Voraussetzung: im Launcher sind mindestens zwei Microsoft-Accounts eingeloggt (einer davon mit
Java Edition). Native Mods laufen mit deinen Rechten und sind nicht sandboxed — genau dafür ist
`--trust-native-code` da.

Horizon's launcher signs several Microsoft accounts in and starts the game with one
of them. `horizon-launcher account session <WHO>` renews another signed-in account's session **for a game that is
already running** and writes the read-once session file it hands over. This mod asks the launcher for that file,
installs it in place of the session the game started with, leaves the server and joins it again. From the next login
on, the session service's join, the profile, the skins and the chat keys belong to the other account.

- The **switch key** (F9 by default, set in the mod's page) opens the mod's own screen: one row per
  signed-in account (`Play as <name>`, `Refresh list`), **+ Add account** for a Microsoft sign-in, and
  the sign-in's code and link right there. Escape closes it. The sign-in's page opens in the browser
  and its code is put on the clipboard, ready to paste; `Copy code …` copies it again.
- **Options → Mods → Account Switch** carries the same rows as a settings page.
- Chat says what is happening (`Account Switch: playing as …`, `joining <server> again`).

Switching costs one reconnect (a second or two): the server sees a different player, so a fresh login is the only
honest way. You can't play two accounts in one game.

## Zoom

The mod also carries a zoom, like OptiFine's: hold the zoom key (C by default, set on the mod's page) to
zoom in 4x, and turn the wheel while holding it to change how far (1.25x to 10x). Turning slows down with
it, as the spyglass does. It fills the game's own `native_zoom`, so the camera, smoothing and FOV easing are
Horizon's, not a copy of them.

**With Horizon's official Zoom mod installed, remove one of the two**: both answer `native_zoom`, and the
first one to answer wins, so the keys would disagree about what they do.

## Minimap and world map

Both are drawn from the mod's own map image: the surface height of every column in reach, in vanilla's map
colours (`MAP_COLORS`), uploaded once into `horizon/modimage` (`native_mod_image`, API 355) and refreshed
when the player moved 8 texels, the zoom changed or 10 seconds passed.

- **Minimap**: the image's middle 64 texels, 72 GUI pixels, in a corner (top right by default; the mod's
  page moves it), with a two-pixel frame and a red dot for the player. Drawn after the hotbar's HUD layer
  (`native_gui_layer_post`, API 139), so it does not replace or double the game's own minimap — the game's
  is off while this mod is installed, because the mod no longer answers `native_minimap`.
- **World map** (`M`): the whole 256-texel image, as large as the window allows, with the player in the
  middle and the scale in blocks per pixel. The wheel zooms out and in, 1 to 16 blocks to a texel — a whole
  4096 blocks across at the farthest. Only loaded chunks have colours; the rest is dark.

With Horizon's official Minimap mod there is no clash any more: that one answers `native_minimap` and draws
the game's minimap, this one draws its own image over the HUD.

## What it needs

- Horizon with native-mod support (the launcher that builds native mods), Minecraft 26.3.
- Two or more Microsoft accounts signed in in the launcher, at least one of them owning Java Edition.
- The launcher's own `account session` command (this revision has it). The game starts the launcher it was started
  by (`HORIZON_LAUNCHER_EXE`), so both are the same copy.

## Install

The bundle is a folder with `horizon-mixin.toml`, the helper `.rs` files and the mod's own crate `core/`:

```sh
horizon-launcher native-build /path/to/account-switch --trust-native-code
```

Native mods run with your user's rights and are not sandboxed; that is what the trust flag is about. The first build
downloads Rust and compiles the game (20–30 minutes); later builds are faster. Then start the instance it built and
open **Options → Mods → Account Switch**.

## How it works

| File | What |
| --- | --- |
| `horizon-mixin.toml` | the bundle: 8 injections, 2 support modules, 1 crate |
| `module.rs` | `crate::game::account_switch`: the launcher calls, the state, the frame/key/settings hooks |
| `slot.rs` | `crate::auth::account_switch`: the session the game plays as |
| `set_current.rs`, `current.rs` | `auth::set_current` installs the session there, `auth::current` answers with it (cancellable HEAD: the original one-shot static is only used while no session is installed) |
| `frame.rs` | once a frame: install a fetched session, leave the server, join it again |
| `key.rs`, `mods.rs`, `settings.rs`, `click.rs` | the key, the Options → Mods row and its page |
| `screen_render.rs`, `screen_click.rs`, `wheel.rs` | the mod's own screen (`native_mod_screen_open`, API 111): its own dark panel and rows drawn with plain fills, one row per account, a scroll lane and the wheel when they don't all fit, its clicks taken |
| `join.rs` | every server join keeps its address, for the switch to rejoin |
| `core/` | the logic without the game: the launcher's JSON, which account is next, the page's rows (`cargo test`) |

Two details worth knowing:

- **Chat keys.** The profile key pair is signed for one account's UUID and a server refuses another account's.
  `slot.rs` drops the cached pair when a session is installed, and the mod calls `auth::prepare_chat_keys(true)`
  right after; the next tick sends the new account's `chat session update`. Without that the login as the second
  account would be kicked on a server with `enforce-secure-profile`.
- **The world.** Leaving a server mid-game is the game's own `leave_server(None)` (what the "Leave server" button and
  `horizon-ctl disconnect` use); joining again is `connect_server(address, name)`, the call behind the server list.
  A world is never saved wrong: single player is only left by its normal save path, and a switch from single player
  just says who is playing — the next server join uses that account.

## Limits

- A switch needs the launcher to reach Microsoft; offline, the sign-in can't be renewed and the mod says so.
- The account must own Java Edition for online-mode servers, as in the launcher.
- If the launcher's refresh invalidates something for the account you are playing, you see it as a normal "invalid
  session" on the next join; signing in again in the launcher fixes it.
- Not tested with split-screen (the mod ignores the window's guests).

## Licence

MIT, like Horizon and horizon-mixins.
