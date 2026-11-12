// No command is exposed to the webview: only Rust (src-tauri) calls the plugin,
// so there are no permission files and the COMMANDS list is empty.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).android_path("android").build();
}
