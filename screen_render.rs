// The mod's own screen, drawn over the world once a frame.
fn horizon_companion_screen_draw(
    game: &crate::game::Game<'_>,
    batch: &mut crate::hud::HudBatch,
    screen: &crate::mod_api::NativeScreenView<'_>,
) {
    crate::game::horizon_companion::draw(game, batch, screen);
}
