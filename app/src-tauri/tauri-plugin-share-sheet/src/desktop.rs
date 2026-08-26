use tauri::{plugin::PluginApi, AppHandle, Runtime};

// Desktop stub — the share sheet is mobile-only. Commands resolve
// immediately without error so `tauri dev` on desktop still works for
// fast UI iteration, even though desktop isn't a shipped target.
pub struct ShareSheet<R: Runtime>(AppHandle<R>);

impl<R: Runtime> ShareSheet<R> {
    pub fn share_file(&self, _file_name: String, _contents: String, _mime_type: String) -> Result<(), String> {
        Ok(())
    }

    pub fn share_text(&self, _text: String) -> Result<(), String> {
        Ok(())
    }
}

pub fn init<R: Runtime, C: serde::de::DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> tauri::Result<ShareSheet<R>> {
    Ok(ShareSheet(app.clone()))
}
