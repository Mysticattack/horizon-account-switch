// The wheel while the zoom key is held: it changes how far.
fn horizon_companion_zoom_wheel(steps: f32, claimed: &mut bool) {
    crate::game::horizon_companion::zoom_wheel(steps, claimed);
}
