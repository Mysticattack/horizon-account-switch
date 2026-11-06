// The mod's row in Options > Mods.
fn horizon_companion_mods(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    crate::game::horizon_companion::listed(mods);
}
