// Once a frame, on the window's own game.
fn account_switch_frame(game: &mut crate::game::Game<'_>) {
    crate::game::account_switch::pump(game);
}
