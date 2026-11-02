// The account this game plays as: the one it was started with, or the one a
// mid-game switch installed (the Account Switch mod). `set_current` and
// `current` hand the session through here, so every join, every profile call
// and the chat keys follow the account that is actually playing.
use std::sync::{Arc, Mutex, MutexGuard};

use super::{AuthSession, CHAT_KEYS};

static SESSION: Mutex<Option<Arc<AuthSession>>> = Mutex::new(None);

fn slot() -> MutexGuard<'static, Option<Arc<AuthSession>>> {
    SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The session the game plays as.
pub fn session() -> Option<Arc<AuthSession>> {
    slot().clone()
}

/// Takes the session: the game's own at start, another account's after a
/// switch. The chat keys belong to the account that signed in before, so they
/// are dropped here and the next prepareChatKeys fetches the new account's:
/// the pair is signed for that account's UUID and a server refuses another's.
pub fn install(session: &AuthSession) {
    // The session the game hands over; a copy is what gets installed, so the
    // caller keeps its own (the hooked `set_current` may still run after this).
    let installed = AuthSession {
        name: session.name.clone(),
        uuid: session.uuid,
        xuid: session.xuid.clone(),
        access_token: session.access_token.clone(),
    };
    *slot() = Some(Arc::new(installed));
    let mut keys = CHAT_KEYS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    keys.current = None;
    keys.next_try = None;
}
