// The session the game plays as: the game's own at start, another account's
// after a switch. Break keeps it out of the one-shot static.
fn horizon_companion_set_current(session: &crate::auth::AuthSession) -> core::ops::ControlFlow<()> {
    crate::auth::horizon_companion::install(session);
    core::ops::ControlFlow::Break(())
}
