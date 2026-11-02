// Who is playing: the installed session, else the game's own (Continue runs it).
fn account_switch_current() -> core::ops::ControlFlow<Option<Arc<AuthSession>>> {
    match crate::auth::account_switch::session() {
        Some(session) => core::ops::ControlFlow::Break(Some(session)),
        None => core::ops::ControlFlow::Continue(()),
    }
}
