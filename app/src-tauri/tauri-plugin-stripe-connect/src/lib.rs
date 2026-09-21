//! Stripe Connect embedded onboarding, presented natively on iOS.
//!
//! Two pieces, because of where the Stripe SDK has to live:
//!
//! - `ios/` — this plugin's Swift package. It does NOT import Stripe. Tauri
//!   compiles plugin packages into a static library, which drops SwiftPM
//!   resource bundles, and the Stripe SDK needs its bundles at runtime.
//! - `app-target/StripeConnectBridge.swift` — compiled into the app target
//!   itself, next to the Stripe SDK that Xcode links there via SwiftPM (the
//!   app's build script adds both). The plugin finds it by class name at
//!   runtime.
//!
//! JS API (native commands, no Rust in between):
//!
//! ```js
//! const onRefresh = new Channel();
//! onRefresh.onmessage = async () => {
//!   const s = await invoke('...fetch a fresh account session...');
//!   await invoke('plugin:stripe-connect|provide_client_secret', { clientSecret: s.clientSecret });
//! };
//! const result = await invoke('plugin:stripe-connect|present_onboarding', {
//!   publishableKey, clientSecret, onRefresh,
//! });
//! // result: { error?: string } — resolves when the user leaves onboarding.
//! ```

use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_stripe_connect);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("stripe-connect")
        .setup(|_app, _api| {
            #[cfg(target_os = "ios")]
            _api.register_ios_plugin(init_plugin_stripe_connect)?;
            Ok(())
        })
        .build()
}
