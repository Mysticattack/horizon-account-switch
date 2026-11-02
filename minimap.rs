// The minimap in the corner, and the big map the map key opens.
fn account_switch_minimap(
    minimap: &mut Option<crate::game::native_hooks::NativeMinimap>,
) {
    crate::game::account_switch::minimap_view(minimap);
}
