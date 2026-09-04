use i18n::{I18nManager, Locale};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

pub mod middleware;

static SUPPORTED_LOCALES: OnceLock<Vec<String>> = OnceLock::new();

/// Shared i18n manager instance.
pub type I18nManagerRef = Arc<RwLock<I18nManager>>;

/// Initialize the shared i18n manager by loading locale dictionaries at runtime.
pub fn init_i18n_manager() -> I18nManagerRef {
    let mut mgr = I18nManager::new(Locale::ZH_CN);
    let mut locales: Vec<String> = Vec::new();

    // Embed locale JSONs at compile time — no runtime filesystem dependency
    let locale_entries: [(&str, &str); 2] = [
        ("en", include_str!("locales/en.json")),
        ("zh-CN", include_str!("locales/zh-CN.json")),
    ];

    for (locale_name, content) in locale_entries {
        if !content.trim().is_empty() {
            if let Err(e) = mgr.load_locale_from_json(&Locale::new(locale_name), content) {
                common::telemetry::warn!("Failed to load locale '{}': {}", locale_name, e);
                continue;
            }
            locales.push(locale_name.to_string());
        }
    }

    locales.sort();
    if let Err(existing) = SUPPORTED_LOCALES.set(locales) {
        common::telemetry::warn!("SUPPORTED_LOCALES already initialized with {:?}", existing);
    }

    Arc::new(RwLock::new(mgr))
}

/// Return the list of supported locales discovered at runtime.
pub fn supported_locales() -> &'static [String] {
    SUPPORTED_LOCALES.get().map(|v| v.as_slice()).unwrap_or(&[])
}
