// The buttons of the mod's settings page.
fn account_switch_settings(mod_id: &str, buttons: &mut Vec<String>) {
    crate::game::account_switch::page(mod_id, buttons);
}
