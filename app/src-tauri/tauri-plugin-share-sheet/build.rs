const COMMANDS: &[&str] = &["share_file", "share_text"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .android_path("android")
        .build();

    // No FileProvider manifest patch needed: Tauri's Android template already
    // declares one (`${applicationId}.fileprovider`, see gen/android's
    // AndroidManifest.xml + res/xml/file_paths.xml) with a cache-path that
    // covers the whole cache dir — exactly where SharePlugin.kt writes the
    // vCard file. SharePlugin.kt reuses that authority directly.
}
