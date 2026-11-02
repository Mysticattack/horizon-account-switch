// The mod's row in Options > Mods.
fn account_switch_mods(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    crate::game::account_switch::listed(mods);
}
