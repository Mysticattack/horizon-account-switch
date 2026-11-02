// A click on one of the page's buttons.
fn account_switch_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    crate::game::account_switch::clicked(game, mod_id, button);
}
