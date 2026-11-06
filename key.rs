// The switch key went down or up.
fn horizon_companion_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::horizon_companion::on_key(game, key, pressed);
}
