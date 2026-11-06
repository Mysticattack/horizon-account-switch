// The minimap Horizon draws itself: the pack's corner, size, zoom and shape.
fn horizon_companion_minimap(
    game: &crate::game::Game<'_>,
    minimap: &mut Option<crate::game::native_hooks::NativeMinimap>,
) {
    crate::game::horizon_companion::minimap(game, minimap);
}
