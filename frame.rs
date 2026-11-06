// Once a frame, on the window's own game.
fn horizon_companion_frame(game: &mut crate::game::Game<'_>) {
    crate::game::horizon_companion::pump(game);
}
