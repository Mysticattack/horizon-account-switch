// A click on one of the settings page's buttons.
fn horizon_companion_page_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    crate::game::horizon_companion::clicked_page(game, mod_id, button);
}
