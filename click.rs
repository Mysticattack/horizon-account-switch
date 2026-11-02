// A click on one of the settings page's buttons.
fn account_switch_page_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    crate::game::account_switch::clicked_page(game, mod_id, button);
}
