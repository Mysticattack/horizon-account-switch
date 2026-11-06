// A key on the mod's name window: Enter, Escape, Backspace.
fn horizon_companion_screen_key(key: &str, canceled: &mut bool) {
    crate::game::horizon_companion::screen_key(key, canceled);
}
