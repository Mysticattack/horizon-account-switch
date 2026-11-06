// Coordinates under the minimap, placed on the engine's own corner stack.
fn horizon_companion_hud(
    game: &crate::game::Game<'_>,
    batch: &mut crate::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    crate::game::horizon_companion::hud_post(game, batch, corners);
}
