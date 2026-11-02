// The mod's own screen, drawn over the world once a frame.
fn account_switch_screen_draw(
    game: &crate::game::Game<'_>,
    batch: &mut crate::hud::HudBatch,
    screen: &crate::mod_api::NativeScreenView<'_>,
) {
    crate::game::account_switch::draw(game, batch, screen);
}
