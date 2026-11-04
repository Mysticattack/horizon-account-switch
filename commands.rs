// The commands the chat box lists.
fn account_switch_commands(commands: &mut Vec<&'static str>) {
    crate::game::account_switch::commands(commands);
}
