// Dragging on the mod's own screen pans the world map.
fn horizon_companion_drag(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    delta: [f32; 2],
    canceled: &mut bool,
) {
    crate::game::horizon_companion::dragged(game, screen, button, delta, canceled);
}
