//! UI strings: `ui/{en,de,fr,es}.json`, embedded at compile time.
//!
//! Each file is a flat JSON object of `"key": "text"` pairs. The crate has
//! no JSON dependency, so a small reader for exactly that shape lives here
//! (string escapes `\" \\ \/ \n \t \r \b \f \uXXXX` are supported; nested
//! values are a build-time data error that the tests report).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::Locale;

/// The UI files, in [`Locale::ALL`] order.
pub const UI_LOCALES: [(Locale, &str); 4] = [
    (Locale::En, include_str!("../ui/en.json")),
    (Locale::De, include_str!("../ui/de.json")),
    (Locale::Fr, include_str!("../ui/fr.json")),
    (Locale::Es, include_str!("../ui/es.json")),
];

/// Parsed tables, one per locale, built on first use.
fn tables() -> &'static [HashMap<&'static str, &'static str>; 4] {
    static TABLES: OnceLock<[HashMap<&'static str, &'static str>; 4]> = OnceLock::new();
    TABLES.get_or_init(|| UI_LOCALES.map(|(_, text)| parse_flat_object(text)))
}

/// The text for `key` in `locale`, if the file has it.
pub(crate) fn lookup(locale: Locale, key: &str) -> Option<&'static str> {
    tables()[locale as usize].get(key).copied()
}

/// Number of keys in a locale's file.
#[cfg(test)]
pub(crate) fn key_count(locale: Locale) -> usize {
    tables()[locale as usize].len()
}

/// Every key of the English file, sorted.
pub fn ui_keys() -> &'static [&'static str] {
    static KEYS: OnceLock<Vec<&'static str>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys: Vec<&'static str> = tables()[Locale::En as usize].keys().copied().collect();
        keys.sort_unstable();
        keys
    })
}

/// Leaks an unknown key once so [`crate::t`] can return it as `&'static str`.
/// Unknown keys are programming errors, so the set stays tiny.
pub(crate) fn intern_key(key: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let interned = INTERNED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = match interned.lock() {
        Ok(m) => m,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(&s) = map.get(key) {
        return s;
    }
    let leaked: &'static str = Box::leak(key.to_owned().into_boxed_str());
    map.insert(key.to_owned(), leaked);
    leaked
}

/// Parses a flat `{"key": "text", ...}` object. Strings without escapes
/// borrow from the source; escaped strings are decoded and leaked (the
/// source is `'static` and read once). Malformed input yields the pairs
/// read so far; the tests compare key sets across locales, so damage is
/// visible.
fn parse_flat_object(src: &'static str) -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    let mut p = Parser {
        src,
        pos: 0,
        bytes: src.as_bytes(),
    };
    p.ws();
    if !p.eat(b'{') {
        return map;
    }
    loop {
        p.ws();
        if p.eat(b'}') {
            break;
        }
        let Some(k) = p.string() else { break };
        p.ws();
        if !p.eat(b':') {
            break;
        }
        p.ws();
        let Some(v) = p.string() else { break };
        map.insert(k, v);
        p.ws();
        if p.eat(b',') {
            continue;
        }
        p.ws();
        let _ = p.eat(b'}');
        break;
    }
    map
}

struct Parser {
    src: &'static str,
    bytes: &'static [u8],
    pos: usize,
}

impl Parser {
    fn ws(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.pos += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.bytes.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// A JSON string starting at the current `"`.
    fn string(&mut self) -> Option<&'static str> {
        if !self.eat(b'"') {
            return None;
        }
        let start = self.pos;
        let mut escaped = false;
        while let Some(&b) = self.bytes.get(self.pos) {
            match b {
                b'"' => {
                    let raw = &self.src[start..self.pos];
                    self.pos += 1;
                    return Some(if escaped {
                        Box::leak(unescape(raw).into_boxed_str())
                    } else {
                        raw
                    });
                }
                b'\\' => {
                    escaped = true;
                    self.pos += 2;
                }
                _ => self.pos += 1,
            }
        }
        None
    }
}

/// Decodes JSON string escapes; an invalid escape is kept literally.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_object_parsing() {
        let m = parse_flat_object(
            "{\n  \"a\": \"x\",\n  \"b.c\": \"say \\\"hi\\\"\\n\",\n  \"u\": \"\\u00e9t\\u00e9\"\n}\n",
        );
        assert_eq!(m.get("a"), Some(&"x"));
        assert_eq!(m.get("b.c"), Some(&"say \"hi\"\n"));
        assert_eq!(m.get("u"), Some(&"été"));
        assert_eq!(m.len(), 3);
        assert!(parse_flat_object("[]").is_empty());
        assert!(parse_flat_object("").is_empty());
        // Trailing comma and nested values stop the reader without panicking.
        assert_eq!(parse_flat_object("{\"a\":\"1\",}").len(), 1);
        assert_eq!(parse_flat_object("{\"a\":{\"b\":\"1\"}}").len(), 0);
    }

    #[test]
    fn unescape_edge_cases() {
        assert_eq!(unescape("a\\qb"), "a\\qb");
        assert_eq!(unescape("end\\"), "end\\");
        assert_eq!(unescape("\\uZZZZ"), "\\uZZZZ");
        assert_eq!(unescape("\\/"), "/");
    }

    #[test]
    fn all_files_parse_as_flat_objects() {
        for (locale, text) in UI_LOCALES {
            let m = parse_flat_object(text);
            assert!(m.len() >= 300, "{locale}: {} keys", m.len());
            // Every key is dotted lower camel case, every value non-empty.
            for (k, v) in &m {
                assert!(
                    k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'),
                    "{locale}: bad key {k}"
                );
                assert!(!v.trim().is_empty(), "{locale}: empty {k}");
            }
        }
        let interned = intern_key("x.y");
        assert_eq!(interned, "x.y");
        assert!(std::ptr::eq(interned, intern_key("x.y")));
    }
}
