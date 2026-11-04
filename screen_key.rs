// A key on the mod's name window: Enter, Escape, Backspace.
fn account_switch_screen_key(key: &str, canceled: &mut bool) {
    crate::game::account_switch::screen_key(key, canceled);
}
