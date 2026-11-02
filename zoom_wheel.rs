// The wheel while the zoom key is held: it changes how far.
fn account_switch_zoom_wheel(steps: f32, claimed: &mut bool) {
    crate::game::account_switch::zoom_wheel(steps, claimed);
}
