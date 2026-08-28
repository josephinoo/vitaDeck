use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use std::collections::HashMap;
use unic_langid::LanguageIdentifier;

const EN: &str = include_str!("../assets/locales/en-US/main.ftl");
const ES: &str = include_str!("../assets/locales/es-ES/main.ftl");
const PT: &str = include_str!("../assets/locales/pt-BR/main.ftl");
const FR: &str = include_str!("../assets/locales/fr-FR/main.ftl");
const IT: &str = include_str!("../assets/locales/it-IT/main.ftl");

const STATIC_KEYS: &[&str] = &[
    "tab-all", "tab-recent", "tab-store", "tab-favorites", "menu", "search", "clear-search",
    "settings", "back", "close", "select", "launch", "install", "details", "collections",
    "remove", "toggle", "view", "rescan", "change", "clean", "purge-music", "purge-all",
    "loading-wait", "loading-scan", "loading-artwork", "search-store", "search-games",
    "collection-title", "confirm-download", "download-question", "download-yes", "cancel",
    "settings-general", "settings-storage", "settings-version", "settings-library", "settings-collections",
    "settings-wifi", "settings-free-ram", "settings-textures", "settings-bgm", "settings-bgm-hint",
    "settings-language", "settings-cache-limit", "settings-cache-hint", "settings-covers", "settings-heroes",
    "settings-music", "settings-total", "settings-orphans", "settings-clean-orphans", "settings-purge-music",
    "settings-purge-all", "empty-games", "empty-log", "no-cover", "status-enabled", "status-disabled", "status-connected",
    "status-offline", "budget-unlimited", "budget-custom", "language-name-en-US", "language-name-es-ES",
    "language-name-pt-BR", "language-name-fr-FR", "language-name-it-IT",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Locale {
    #[serde(rename = "en-US")]
    EnUs,
    #[serde(rename = "es-ES")]
    EsEs,
    #[serde(rename = "pt-BR")]
    PtBr,
    #[serde(rename = "fr-FR")]
    FrFr,
    #[serde(rename = "it-IT")]
    ItIt,
}

impl Default for Locale {
    fn default() -> Self { Self::EnUs }
}

impl Locale {
    pub fn next(self) -> Self {
        match self { Self::EnUs => Self::EsEs, Self::EsEs => Self::PtBr, Self::PtBr => Self::FrFr, Self::FrFr => Self::ItIt, Self::ItIt => Self::EnUs }
    }
    fn tag(self) -> &'static str {
        match self { Self::EnUs => "en-US", Self::EsEs => "es-ES", Self::PtBr => "pt-BR", Self::FrFr => "fr-FR", Self::ItIt => "it-IT" }
    }
    fn source(self) -> &'static str {
        match self { Self::EnUs => EN, Self::EsEs => ES, Self::PtBr => PT, Self::FrFr => FR, Self::ItIt => IT }
    }
}

pub struct Localizer {
    bundle: FluentBundle<FluentResource>,
    fallback: FluentBundle<FluentResource>,
    static_text: HashMap<&'static str, String>,
}

impl Localizer {
    pub fn new(locale: Locale) -> Self {
        let bundle = Self::bundle(locale.tag(), locale.source());
        let fallback = Self::bundle("en-US", EN);
        let mut this = Self { bundle, fallback, static_text: HashMap::new() };
        for key in STATIC_KEYS { let text = this.format(key, None); this.static_text.insert(key, text); }
        this
    }
    fn bundle(tag: &str, source: &str) -> FluentBundle<FluentResource> {
        let locale: LanguageIdentifier = tag.parse().expect("valid built-in locale");
        let resource = FluentResource::try_new(source.to_owned()).expect("valid built-in FTL");
        let mut bundle = FluentBundle::new(vec![locale]);
        bundle.set_use_isolating(false);
        bundle.add_resource(resource).expect("unique FTL keys");
        bundle
    }
    pub fn text(&self, key: &str) -> &str { self.static_text.get(key).map(String::as_str).unwrap_or("") }
    pub fn format(&self, key: &str, args: Option<&FluentArgs<'_>>) -> String {
        for bundle in [&self.bundle, &self.fallback] {
            if let Some(message) = bundle.get_message(key) {
                if let Some(pattern) = message.value() {
                    let mut errors = Vec::new();
                    return bundle.format_pattern(pattern, args, &mut errors).into_owned();
                }
            }
        }
        key.to_owned()
    }
    pub fn format_one(&self, key: &str, name: &'static str, value: impl Into<fluent_bundle::FluentValue<'static>>) -> String {
        let mut args = FluentArgs::new();
        args.set(name, value);
        self.format(key, Some(&args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_builtin_locale_has_static_messages() {
        for locale in [Locale::EnUs, Locale::EsEs, Locale::PtBr, Locale::FrFr, Locale::ItIt] {
            let localizer = Localizer::new(locale);
            for key in STATIC_KEYS {
                assert!(!localizer.text(key).is_empty(), "{locale:?} missing {key}");
            }
        }
    }
    #[test]
    fn locale_cycles() { assert_eq!(Locale::ItIt.next(), Locale::EnUs); }
}
