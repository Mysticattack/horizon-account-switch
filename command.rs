// The chat command /wp: waypoints by name.
fn horizon_companion_command(
    game: &mut crate::game::Game<'_>,
    command: &str,
    ran: &mut bool,
) {
    crate::game::horizon_companion::command(game, command, ran);
}
