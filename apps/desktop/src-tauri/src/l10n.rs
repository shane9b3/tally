//! Native-menu localisation (§4 item 47, shell part) and the UI-language
//! commands: `set_ui_language(lang)` stores the language in `prefs.json` and
//! rebuilds the native menu with `tally_l10n::t()` labels (the `cmd.<id>`
//! and `menu.*` keys of `ui/*.json`); `ui_strings(lang)` hands the webview a
//! table (it ships its own copies but can refresh from here);
//! `localized_function_names(lang)` returns the canonical ⇄ localized pairs.

use std::collections::BTreeMap;

use tally_api::ApiError;
use tally_l10n::Locale;
use tauri::{AppHandle, Runtime};

use crate::commands::CmdResult;
use crate::prefs;

/// The locale of a language code or tag (`"de"`, `"de-DE"`); unknown → English.
pub fn locale_of(lang: &str) -> Locale {
    Locale::parse(lang).unwrap_or(Locale::En)
}

/// The UI locale stored in the prefs.
pub fn current_locale<R: Runtime>(app: &AppHandle<R>) -> Locale {
    locale_of(&prefs::ui_language(&prefs::load(app)))
}

/// The label of a menu item: English keeps the menu's own label (with its
/// `…` and title case); other locales take `cmd.<id>` from the UI table and
/// keep the fallback when the key is missing.
pub fn item_label(locale: Locale, id: &str, fallback: &str) -> String {
    if locale == Locale::En {
        return fallback.to_owned();
    }
    let key = format!("cmd.{id}");
    if !tally_l10n::has_ui_key(&key) {
        return fallback.to_owned();
    }
    let text = tally_l10n::t(locale, &key);
    // The English table and the menu differ in the trailing ellipsis; follow the menu.
    let base = text.trim_end_matches('…');
    if fallback.ends_with('…') {
        format!("{base}…")
    } else {
        base.to_owned()
    }
}

/// Submenu titles map to `menu.*` keys.
pub fn submenu_label(locale: Locale, title: &str) -> String {
    let key = match title {
        "File" => "menu.file",
        "Edit" => "menu.edit",
        "View" => "menu.view",
        "Insert" => "menu.insert",
        "Format" => "menu.format",
        "Sheet" => "menu.sheet",
        "Data" => "menu.data",
        "Tools" => "menu.tools",
        "Window" => "menu.window",
        "Help" => "menu.help",
        "Export" => "menu.file.export",
        "Open Recent" => "menu.file.recent",
        "Number Format" => "menu.format.numberFormat",
        "Filter" => "menu.data.filter",
        "Group and Outline" => "menu.data.group",
        "DataPilot" => "menu.data.dataPilot",
        "Styles" => "menu.format.styles",
        "Chart" => "chart.title",
        "Detective" => "cmd.tools.detective",
        "Locale" => "options.language",
        _ => return title.to_owned(),
    };
    if locale == Locale::En || !tally_l10n::has_ui_key(key) {
        return title.to_owned();
    }
    tally_l10n::t(locale, key).to_owned()
}

/// Every UI string of a locale (the webview's `t()` table).
pub fn ui_table(locale: Locale) -> BTreeMap<&'static str, &'static str> {
    tally_l10n::ui_keys()
        .iter()
        .map(|k| (*k, tally_l10n::t(locale, k)))
        .collect()
}

/// `set_ui_language(lang)`: stores the language and rebuilds the menu.
#[tauri::command]
pub async fn set_ui_language(app: AppHandle, lang: String) -> CmdResult<()> {
    crate::limits::text_max("the language", &lang, crate::limits::MAX_LOCALE)?;
    let locale = locale_of(&lang);
    let next = prefs::with_string_key(&prefs::load(&app), "uiLanguage", locale.language())?;
    prefs::store(&app, &next)?;
    let menu = crate::menu::build_localized(&app, locale)
        .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("menu: {e}")))?;
    app.set_menu(menu)
        .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("menu: {e}")))?;
    Ok(())
}

/// `ui_strings(lang)`: the flat `key → text` table of a locale.
#[tauri::command]
pub async fn ui_strings(lang: String) -> CmdResult<BTreeMap<&'static str, &'static str>> {
    crate::limits::text_max("the language", &lang, crate::limits::MAX_LOCALE)?;
    Ok(ui_table(locale_of(&lang)))
}

/// `localized_function_names(lang)`: `(canonical, localized)` pairs (empty for English).
#[tauri::command]
pub async fn localized_function_names(lang: String) -> CmdResult<Vec<(String, String)>> {
    crate::limits::text_max("the language", &lang, crate::limits::MAX_LOCALE)?;
    Ok(tally_l10n::function_names(locale_of(&lang))
        .iter()
        .map(|(c, l)| ((*c).to_owned(), (*l).to_owned()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_locale() {
        assert_eq!(item_label(Locale::En, "file.open", "Open…"), "Open…");
        assert_eq!(item_label(Locale::De, "edit.undo", "Undo"), "Rückgängig");
        assert_eq!(item_label(Locale::Fr, "file.open", "Open…"), "Ouvrir…");
        assert_eq!(item_label(Locale::De, "no.such", "Keep me"), "Keep me");
        assert_eq!(submenu_label(Locale::De, "File"), "Datei");
        assert_eq!(submenu_label(Locale::De, "Edit"), "Bearbeiten");
        assert_eq!(submenu_label(Locale::En, "File"), "File");
        assert_eq!(submenu_label(Locale::Es, "Nope"), "Nope");
        assert_eq!(locale_of("de-AT"), Locale::De);
        assert_eq!(locale_of("xx"), Locale::En);
    }

    #[test]
    fn tables_and_names() {
        let t = ui_table(Locale::De);
        assert_eq!(t.get("menu.file"), Some(&"Datei"));
        assert!(t.len() >= 600);
        assert!(tally_l10n::function_names(Locale::De)
            .iter()
            .any(|(c, l)| *c == "SUM" && *l == "SUMME"));
    }
}
