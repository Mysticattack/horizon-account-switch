// The HUD's layers, after one has drawn: the minimap goes over the hotbar's.
fn horizon_companion_hud(
    game: &crate::game::Game<'_>,
    layer: &str,
    batch: &mut crate::hud::HudBatch,
) {
    crate::game::horizon_companion::hud_post(game, layer, batch);
}
