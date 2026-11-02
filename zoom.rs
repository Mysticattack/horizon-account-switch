// The zoom in effect, unless another mod's came first.
fn account_switch_zoom(zoom: &mut Option<crate::game::native_hooks::NativeZoom>) {
    crate::game::account_switch::zoom_view(zoom);
}
