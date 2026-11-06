// A mouse button over the mod's own screen: a row acts, or the press goes on.
fn horizon_companion_screen_click(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    crate::game::horizon_companion::clicked(game, screen, button, canceled);
}
