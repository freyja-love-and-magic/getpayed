use serde::Serialize;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_share_sheet);

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.planetnine.gelder.sharesheet";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareFileArgs {
    file_name: String,
    contents: String,
    mime_type: String,
}

#[derive(Serialize)]
struct ShareTextArgs {
    text: String,
}

pub struct ShareSheet<R: Runtime>(PluginHandle<R>);

unsafe impl<R: Runtime> Send for ShareSheet<R> {}
unsafe impl<R: Runtime> Sync for ShareSheet<R> {}

impl<R: Runtime> ShareSheet<R> {
    pub fn share_file(&self, file_name: String, contents: String, mime_type: String) -> Result<(), String> {
        self.0
            .run_mobile_plugin(
                "shareFile",
                ShareFileArgs { file_name, contents, mime_type },
            )
            .map_err(|e| e.to_string())
    }

    pub fn share_text(&self, text: String) -> Result<(), String> {
        self.0
            .run_mobile_plugin("shareText", ShareTextArgs { text })
            .map_err(|e| e.to_string())
    }
}

pub fn init<R: Runtime, C: serde::de::DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<ShareSheet<R>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_share_sheet)?;
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "SharePlugin")?;

    Ok(ShareSheet(handle))
}
