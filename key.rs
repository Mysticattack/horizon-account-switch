// The switch key went down or up.
fn account_switch_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::account_switch::on_key(game, key, pressed);
}
