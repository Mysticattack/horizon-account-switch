// A server is being joined: keep its address for a switch to rejoin.
fn account_switch_join(address: &str, name: &str) {
    crate::game::account_switch::remember_server(address, name);
}
