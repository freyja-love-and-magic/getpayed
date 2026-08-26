use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

#[cfg(any(target_os = "ios", target_os = "android"))]
mod mobile;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod desktop;

#[cfg(any(target_os = "ios", target_os = "android"))]
use mobile::ShareSheet;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use desktop::ShareSheet;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareFileArgs {
    pub file_name: String,
    pub contents: String,
    pub mime_type: String,
}

/// Writes `contents` to a temp/cache file named `file_name` and presents the
/// native OS share sheet for it. Never touches vCard formatting logic — the
/// frontend passes the already-generated vCard string straight through.
#[tauri::command]
async fn share_file<R: Runtime>(
    app: tauri::AppHandle<R>,
    file_name: String,
    contents: String,
    mime_type: String,
) -> Result<(), String> {
    app.state::<ShareSheet<R>>()
        .share_file(file_name, contents, mime_type)
        .map_err(|e| e.to_string())
}

/// Presents the native OS share sheet for a plain string — used for sharing
/// a URL as an actual link (tappable, link-previewed) rather than wrapped in
/// a file, which is what `share_file` would produce.
#[tauri::command]
async fn share_text<R: Runtime>(app: tauri::AppHandle<R>, text: String) -> Result<(), String> {
    app.state::<ShareSheet<R>>()
        .share_text(text)
        .map_err(|e| e.to_string())
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("share-sheet")
        .invoke_handler(tauri::generate_handler![share_file, share_text])
        .setup(|app, api| {
            #[cfg(any(target_os = "ios", target_os = "android"))]
            let share_sheet = mobile::init(app, api)?;
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            let share_sheet = desktop::init(app, api)?;
            app.manage(share_sheet);
            Ok(())
        })
        .build()
}
