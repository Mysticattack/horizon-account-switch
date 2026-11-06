// The wheel over the mod's own screen: the list scrolls when it doesn't fit.
fn horizon_companion_wheel(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) {
    crate::game::horizon_companion::wheel(game, screen, scroll, canceled);
}
