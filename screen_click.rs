// A mouse button over the mod's own screen: a row acts, or the press goes on.
fn account_switch_screen_click(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    crate::game::account_switch::clicked(game, screen, button, canceled);
}
