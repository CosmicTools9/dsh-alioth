/// Trait for errors that support internationalization.
///
/// Implement this trait on your custom error types to expose a stable
/// message key that the frontend can translate.
pub trait I18nError {
    /// The translation key for this error variant.
    fn message_key(&self) -> &str;

    /// Optional additional context parameters for interpolation.
    fn context(&self) -> Option<std::collections::HashMap<String, String>> {
        None
    }
}
