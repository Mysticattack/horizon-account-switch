// A mod message from the server: a waypoint another player shared.
fn account_switch_net_from_server(
    game: &mut crate::game::Game<'_>,
    channel: &str,
    data: &[u8],
) {
    crate::game::account_switch::shared(game, channel, data);
}
