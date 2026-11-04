// A character typed on the mod's name window.
fn account_switch_typed(c: char, canceled: &mut bool) {
    crate::game::account_switch::typed(c, canceled);
}
