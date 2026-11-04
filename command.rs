// The chat command /wp: waypoints by name.
fn account_switch_command(
    game: &mut crate::game::Game<'_>,
    command: &str,
    ran: &mut bool,
) {
    crate::game::account_switch::command(game, command, ran);
}
