// The session the game plays as: the game's own at start, another account's
// after a switch. Break keeps it out of the one-shot static.
fn account_switch_set_current(session: &crate::auth::AuthSession) -> core::ops::ControlFlow<()> {
    crate::auth::account_switch::install(session);
    core::ops::ControlFlow::Break(())
}
