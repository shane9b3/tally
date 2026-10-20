//! Localisation data for Tally: function names, boolean literals and UI
//! strings for en-US, de-DE, fr-FR and es-ES.
//!
//! - [`localized_name`] / [`canonical_name`]: canonical (English) function
//!   name ⇄ localized spelling, per locale (`SUM` ⇄ `SUMME`/`SOMME`/`SUMA`).
//!   Names identical to English are not stored; both directions fall back
//!   to the canonical list. Lookups are case-insensitive and, as a second
//!   chance, accent-insensitive (`DIA` finds `DÍA` and vice versa).
//! - [`localized_bool`] / [`parse_bool`]: `TRUE`/`FALSE` literals.
//! - [`t`]: UI strings from `ui/{en,de,fr,es}.json`, falling back to
//!   English and then to the key itself.
//!
//! The formula dialect (argument separator, decimal separator) is the
//! parser's business (`tally-formula`); this crate only supplies names.
//! See `docs/l10n.md`. No dependencies beyond `serde`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub mod names;
mod ui;

pub use ui::{ui_keys, UI_LOCALES};

/// A supported UI / function-name locale.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Locale {
    /// English (United States): the canonical names.
    #[serde(rename = "en-US")]
    En,
    /// German (Germany).
    #[serde(rename = "de-DE")]
    De,
    /// French (France).
    #[serde(rename = "fr-FR")]
    Fr,
    /// Spanish (Spain).
    #[serde(rename = "es-ES")]
    Es,
}

impl Locale {
    /// Every supported locale, English first.
    pub const ALL: [Locale; 4] = [Locale::En, Locale::De, Locale::Fr, Locale::Es];

    /// The BCP 47 tag (`"de-DE"`).
    pub const fn tag(self) -> &'static str {
        match self {
            Locale::En => "en-US",
            Locale::De => "de-DE",
            Locale::Fr => "fr-FR",
            Locale::Es => "es-ES",
        }
    }

    /// The language code (`"de"`), also the UI file name.
    pub const fn language(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::De => "de",
            Locale::Fr => "fr",
            Locale::Es => "es",
        }
    }

    /// Parses a tag or language code, case-insensitively, by language:
    /// `"de"`, `"de-DE"`, `"de_AT"` and `"de-CH"` are all [`Locale::De`].
    /// Unknown languages are `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let lang = s
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match lang.as_str() {
            "en" => Some(Locale::En),
            "de" => Some(Locale::De),
            "fr" => Some(Locale::Fr),
            "es" => Some(Locale::Es),
            _ => None,
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// The supported locales, English first.
pub fn locales() -> &'static [Locale] {
    &Locale::ALL
}

/// The `(canonical, localized)` table of a locale (empty for English).
pub fn function_names(locale: Locale) -> &'static [(&'static str, &'static str)] {
    match locale {
        Locale::En => &[],
        Locale::De => names::de::NAMES,
        Locale::Fr => names::fr::NAMES,
        Locale::Es => names::es::NAMES,
    }
}

/// Canonical names whose spelling in `locale` has not been verified against
/// an AOO l10n checkout (see `docs/l10n.md`).
pub fn unverified(locale: Locale) -> &'static [&'static str] {
    match locale {
        Locale::En => &[],
        Locale::De => names::de::UNVERIFIED,
        Locale::Fr => names::fr::UNVERIFIED,
        Locale::Es => names::es::UNVERIFIED,
    }
}

/// Upper-cases a name for table lookup (Unicode-aware: `ä` → `Ä`).
fn key(s: &str) -> String {
    s.trim().to_uppercase()
}

/// Strips Latin-1 accents (`Ä` → `A`, `É` → `E`) but keeps `Ñ`, which is a
/// letter of its own in Spanish names.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
            'È' | 'É' | 'Ê' | 'Ë' => 'E',
            'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
            'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
            'Ç' => 'C',
            'Ý' => 'Y',
            c => c,
        })
        .collect()
}

/// Lookup tables of one locale, built on first use.
struct Tables {
    /// Canonical name → localized name (only the names that differ).
    to_local: HashMap<&'static str, &'static str>,
    /// Upper-cased localized or canonical name → canonical name.
    to_canonical: HashMap<String, &'static str>,
    /// Accent-folded version of `to_canonical`.
    folded: HashMap<String, &'static str>,
}

fn tables(locale: Locale) -> &'static Tables {
    static TABLES: OnceLock<[Tables; 4]> = OnceLock::new();
    let all = TABLES.get_or_init(|| Locale::ALL.map(build_tables));
    &all[locale as usize]
}

fn build_tables(locale: Locale) -> Tables {
    let mut to_local = HashMap::new();
    let mut to_canonical = HashMap::new();
    for &(canonical, localized) in function_names(locale) {
        to_local.insert(canonical, localized);
        to_canonical.insert(key(localized), canonical);
    }
    for &canonical in names::CANONICAL {
        if !to_local.contains_key(canonical) {
            to_canonical.insert(key(canonical), canonical);
        }
    }
    let folded = to_canonical.iter().map(|(k, v)| (fold(k), *v)).collect();
    Tables {
        to_local,
        to_canonical,
        folded,
    }
}

/// The localized spelling of a canonical function name; the name itself
/// when the locale spells it like English or the name is unknown.
/// Matching of `canonical` is case-insensitive.
pub fn localized_name(locale: Locale, canonical: &str) -> &'static str {
    let t = tables(locale);
    let k = key(canonical);
    if let Some(&local) = t.to_local.get(k.as_str()) {
        return local;
    }
    t.to_canonical.get(&k).copied().unwrap_or_else(|| {
        names::CANONICAL
            .iter()
            .copied()
            .find(|c| *c == k)
            .unwrap_or("")
    })
}

/// The canonical name of a function spelled in `locale` (case-insensitive;
/// accent-insensitive as a fallback). Names that the locale spells like
/// English resolve too. `None` for an unknown name.
pub fn canonical_name(locale: Locale, name: &str) -> Option<&'static str> {
    let t = tables(locale);
    let k = key(name);
    t.to_canonical
        .get(&k)
        .or_else(|| t.folded.get(&fold(&k)))
        .copied()
}

/// The `TRUE` / `FALSE` literal of a locale, as `(true, false)`.
pub const fn bool_literals(locale: Locale) -> (&'static str, &'static str) {
    match locale {
        Locale::En => ("TRUE", "FALSE"),
        Locale::De => ("WAHR", "FALSCH"),
        Locale::Fr => ("VRAI", "FAUX"),
        Locale::Es => ("VERDADERO", "FALSO"),
    }
}

/// The localized `TRUE` or `FALSE` literal.
pub const fn localized_bool(locale: Locale, value: bool) -> &'static str {
    let (t, f) = bool_literals(locale);
    if value {
        t
    } else {
        f
    }
}

/// Parses a boolean literal of `locale` (case-insensitive); the English
/// literals are accepted in every locale, as in AOO.
pub fn parse_bool(locale: Locale, text: &str) -> Option<bool> {
    let k = key(text);
    let (t, f) = bool_literals(locale);
    if k == t || k == "TRUE" {
        Some(true)
    } else if k == f || k == "FALSE" {
        Some(false)
    } else {
        None
    }
}

/// A UI string by key (`"menu.file"`, `"cmd.edit.undo"`, `"dialog.ok"`).
/// Missing translations fall back to English, then to the key itself, so
/// a typo shows up on screen instead of panicking.
pub fn t(locale: Locale, key: &str) -> &'static str {
    ui::lookup(locale, key)
        .or_else(|| ui::lookup(Locale::En, key))
        .unwrap_or_else(|| ui::intern_key(key))
}

/// Whether `key` exists in the English UI table.
pub fn has_ui_key(key: &str) -> bool {
    ui::lookup(Locale::En, key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn locale_parsing_and_tags() {
        assert_eq!(Locale::parse("de"), Some(Locale::De));
        assert_eq!(Locale::parse("DE-at"), Some(Locale::De));
        assert_eq!(Locale::parse("fr_CA.UTF-8"), Some(Locale::Fr));
        assert_eq!(Locale::parse("es-419"), Some(Locale::Es));
        assert_eq!(Locale::parse("en-GB"), Some(Locale::En));
        assert_eq!(Locale::parse("pt-BR"), None);
        assert_eq!(Locale::De.tag(), "de-DE");
        assert_eq!(Locale::De.to_string(), "de-DE");
        assert_eq!(locales().len(), 4);
        assert_eq!(locales()[0], Locale::En);
    }

    #[test]
    fn spot_checks() {
        for (canon, de, fr, es) in [
            ("SUM", "SUMME", "SOMME", "SUMA"),
            ("AVERAGE", "MITTELWERT", "MOYENNE", "PROMEDIO"),
            ("IF", "WENN", "SI", "SI"),
            ("VLOOKUP", "SVERWEIS", "RECHERCHEV", "BUSCARV"),
            ("COUNTIF", "ZÄHLENWENN", "NB.SI", "CONTAR.SI"),
            ("TODAY", "HEUTE", "AUJOURDHUI", "HOY"),
            ("YEAR", "JAHR", "ANNEE", "AÑO"),
            (
                "GETPIVOTDATA",
                "PIVOTDATENZUORDNEN",
                "EXTRAIRE.DONNEES.PILOTE",
                "IMPORTARDATOSDINAMICOS",
            ),
            (
                "MULTIPLE.OPERATIONS",
                "MEHRFACH.OPERATIONEN",
                "OPERATIONS.MULTIPLES",
                "OPERACION.MULTIPLE",
            ),
            ("MAX", "MAX", "MAX", "MAX"),
        ] {
            assert_eq!(localized_name(Locale::En, canon), canon);
            assert_eq!(localized_name(Locale::De, canon), de, "{canon} de");
            assert_eq!(localized_name(Locale::Fr, canon), fr, "{canon} fr");
            assert_eq!(localized_name(Locale::Es, canon), es, "{canon} es");
            assert_eq!(canonical_name(Locale::De, de), Some(canon));
            assert_eq!(canonical_name(Locale::Fr, fr), Some(canon));
            assert_eq!(canonical_name(Locale::Es, es), Some(canon));
        }
        // Case-insensitive, accent-tolerant, whitespace-tolerant.
        assert_eq!(canonical_name(Locale::De, "summe"), Some("SUM"));
        assert_eq!(canonical_name(Locale::De, "zählenwenn"), Some("COUNTIF"));
        assert_eq!(canonical_name(Locale::De, "ZAHLENWENN"), Some("COUNTIF"));
        assert_eq!(canonical_name(Locale::Es, "DÍA"), Some("DAY"));
        assert_eq!(canonical_name(Locale::Es, " año "), Some("YEAR"));
        assert_eq!(canonical_name(Locale::Fr, "Somme"), Some("SUM"));
        assert_eq!(localized_name(Locale::De, "sum"), "SUMME");
        // Unknown names.
        assert_eq!(canonical_name(Locale::De, "NOPE"), None);
        assert_eq!(
            canonical_name(Locale::De, "SUMA"),
            None,
            "Spanish name in German"
        );
        assert_eq!(localized_name(Locale::De, "NOPE"), "");
        // Names spelled like English resolve in every locale; German
        // spellings do not resolve in French.
        assert_eq!(canonical_name(Locale::Fr, "MAX"), Some("MAX"));
        assert_eq!(canonical_name(Locale::Fr, "SUMME"), None);
        // Excel-compatible clash handled per locale: Spanish FIXED is DECIMAL.
        assert_eq!(canonical_name(Locale::Es, "DECIMAL"), Some("FIXED"));
        assert_eq!(canonical_name(Locale::Es, "CONV.DECIMAL"), Some("DECIMAL"));
    }

    #[test]
    fn bijective_per_locale() {
        let canonical: HashSet<&str> = names::CANONICAL.iter().copied().collect();
        assert_eq!(
            canonical.len(),
            names::CANONICAL.len(),
            "duplicate canonical names"
        );
        for locale in locales().iter().copied() {
            let table = function_names(locale);
            let mut seen_canonical = HashSet::new();
            let mut seen_localized = HashSet::new();
            for &(c, l) in table {
                assert!(canonical.contains(c), "{locale}: {c} is not canonical");
                assert!(seen_canonical.insert(c), "{locale}: {c} listed twice");
                assert!(seen_localized.insert(key(l)), "{locale}: {l} used twice");
                assert_ne!(key(c), key(l), "{locale}: {c} maps to itself");
                assert_eq!(l, l.trim(), "{locale}: {l} has whitespace");
                assert!(!l.is_empty(), "{locale}: {c} empty");
            }
            // A localized name never collides with a canonical name that the
            // locale keeps unchanged.
            for &(c, l) in table {
                if let Some(other) = canonical.get(key(l).as_str()) {
                    assert!(
                        seen_canonical.contains(other),
                        "{locale}: {c} -> {l} shadows unchanged {other}"
                    );
                }
            }
            // Round trips over the whole canonical list.
            for &c in names::CANONICAL {
                let l = localized_name(locale, c);
                assert!(!l.is_empty(), "{locale}: {c} has no localized name");
                assert_eq!(canonical_name(locale, l), Some(c), "{locale}: {c} -> {l}");
            }
        }
    }

    #[test]
    fn coverage_counts() {
        assert_eq!(names::CANONICAL.len(), 408);
        assert!(function_names(Locale::En).is_empty());
        for (locale, min) in [(Locale::De, 300), (Locale::Fr, 300), (Locale::Es, 300)] {
            let n = function_names(locale).len();
            assert!(n >= min, "{locale}: only {n} localized names");
            for u in unverified(locale) {
                assert!(
                    names::CANONICAL.contains(u),
                    "{locale}: UNVERIFIED {u} unknown"
                );
            }
            assert!(
                unverified(locale).len() <= 20,
                "{locale}: too many unverified"
            );
        }
    }

    #[test]
    fn booleans() {
        assert_eq!(localized_bool(Locale::De, true), "WAHR");
        assert_eq!(localized_bool(Locale::Fr, false), "FAUX");
        assert_eq!(localized_bool(Locale::Es, true), "VERDADERO");
        assert_eq!(localized_bool(Locale::En, false), "FALSE");
        assert_eq!(parse_bool(Locale::De, "falsch"), Some(false));
        assert_eq!(parse_bool(Locale::Es, "Verdadero"), Some(true));
        assert_eq!(parse_bool(Locale::Fr, "TRUE"), Some(true));
        assert_eq!(parse_bool(Locale::Fr, "WAHR"), None);
    }

    #[test]
    fn serde_tags() {
        use serde::de::IntoDeserializer;
        for locale in locales().iter().copied() {
            let back: Result<Locale, serde::de::value::Error> =
                Locale::deserialize(locale.tag().into_deserializer());
            assert_eq!(back.ok(), Some(locale));
        }
        let bad: Result<Locale, serde::de::value::Error> =
            Locale::deserialize("de".into_deserializer());
        assert!(bad.is_err(), "only full tags are accepted by serde");
    }

    #[test]
    fn ui_strings() {
        assert_eq!(t(Locale::En, "dialog.ok"), "OK");
        assert_eq!(t(Locale::De, "dialog.cancel"), "Abbrechen");
        assert_eq!(t(Locale::Fr, "dialog.cancel"), "Annuler");
        assert_eq!(t(Locale::Es, "dialog.cancel"), "Cancelar");
        assert_eq!(t(Locale::De, "menu.file"), "Datei");
        assert_eq!(t(Locale::De, "cmd.edit.undo"), "Rückgängig");
        assert_eq!(t(Locale::Fr, "cmd.edit.undo"), "Annuler");
        assert_eq!(t(Locale::Es, "cmd.edit.undo"), "Deshacer");
        // Unknown key: the key itself.
        assert_eq!(t(Locale::De, "no.such.key"), "no.such.key");
        assert!(has_ui_key("menu.file"));
        assert!(!has_ui_key("no.such.key"));
    }

    #[test]
    fn every_ui_key_in_every_language() {
        let keys = ui_keys();
        assert!(keys.len() >= 300, "only {} UI keys", keys.len());
        for locale in locales().iter().copied() {
            for k in keys {
                let v = ui::lookup(locale, k);
                assert!(v.is_some(), "{locale}: missing UI key {k}");
                assert!(!v.unwrap_or("").trim().is_empty(), "{locale}: empty {k}");
            }
            // No key beyond the English set either.
            assert_eq!(ui::key_count(locale), keys.len(), "{locale}: extra keys");
        }
    }
}
