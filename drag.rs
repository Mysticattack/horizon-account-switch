// Dragging on the mod's own screen pans the world map.
fn account_switch_drag(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    delta: [f32; 2],
    canceled: &mut bool,
) {
    crate::game::account_switch::dragged(game, screen, button, delta, canceled);
}
