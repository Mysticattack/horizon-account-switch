// The zoom in effect, unless another mod's came first.
fn horizon_companion_zoom(zoom: &mut Option<crate::game::native_hooks::NativeZoom>) {
    crate::game::horizon_companion::zoom_view(zoom);
}
