// The minimap Horizon draws itself: the pack's corner, size, zoom and shape.
fn account_switch_minimap(
    game: &crate::game::Game<'_>,
    minimap: &mut Option<crate::game::native_hooks::NativeMinimap>,
) {
    crate::game::account_switch::minimap(game, minimap);
}
