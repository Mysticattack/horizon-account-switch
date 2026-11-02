// The HUD's layers, after one has drawn: the minimap goes over the hotbar's.
fn account_switch_hud(
    game: &crate::game::Game<'_>,
    layer: &str,
    batch: &mut crate::hud::HudBatch,
) {
    crate::game::account_switch::hud_post(game, layer, batch);
}
