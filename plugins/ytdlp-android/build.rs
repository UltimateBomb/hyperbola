// The plugin is called from Rust only — the web layer talks to the app's own
// commands — so it exposes no JS commands of its own.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
