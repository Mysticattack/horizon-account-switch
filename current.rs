// Who is playing: the installed session, else the game's own (Continue runs it).
fn horizon_companion_current() -> core::ops::ControlFlow<Option<Arc<AuthSession>>> {
    match crate::auth::horizon_companion::session() {
        Some(session) => core::ops::ControlFlow::Break(Some(session)),
        None => core::ops::ControlFlow::Continue(()),
    }
}
