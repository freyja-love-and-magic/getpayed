// No Rust commands: JS invokes these straight through to the Swift plugin
// (Tauri forwards any command a plugin's Rust side doesn't handle to its
// native side). Listed here so Tauri generates allow-* permissions for them.
const COMMANDS: &[&str] = &["present_onboarding", "provide_client_secret"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .build();
}
