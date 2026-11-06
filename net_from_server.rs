// A mod message from the server: a waypoint another player shared.
fn horizon_companion_net_from_server(
    game: &mut crate::game::Game<'_>,
    channel: &str,
    data: &[u8],
) {
    crate::game::horizon_companion::shared(game, channel, data);
}
