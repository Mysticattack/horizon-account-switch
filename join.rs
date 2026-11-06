// A server is being joined: keep its address for a switch to rejoin.
fn horizon_companion_join(address: &str, name: &str) {
    crate::game::horizon_companion::remember_server(address, name);
}
