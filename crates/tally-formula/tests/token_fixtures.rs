//! Shared lexer fixtures in `tests/fixtures/tokens/*.json`, consumed by this
//! test and by the future TypeScript lexer.
//!
//! Format: `{ "src": "...", "dialect": "calc-a1" | "open-formula",
//! "tokens": [{ "kind": "CellRef", "text": "A1", "start": 1, "end": 3 }] }`
//! where `start`/`end` are UTF-8 byte offsets into `src` and `kind` is
//! `TokenKind::name()`.
//!
//! Regenerate after an intended lexer change with
//! `TALLY_REGEN_TOKEN_FIXTURES=1 cargo test -p tally-formula --test token_fixtures`.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::cases::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tally_formula::{tokenize, Dialect, TokenKind};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Tok {
    kind: String,
    text: String,
    start: usize,
    end: usize,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Fixture {
    src: String,
    dialect: String,
    tokens: Vec<Tok>,
}

/// Extra lexer-focused inputs (no leading `=`, odd spacing, unicode, ...).
const LEXER: &[&str] = &[
    "Sheet1.A1",
    "$Sheet1.$A$1",
    "'My Sheet'.A1",
    "Sheet1!A1",
    "'My Sheet'!A1",
    "A1:B2",
    "A:A",
    "1:1",
    "$A$1:$B$2",
    "Sheet1.A1:Sheet3.B2",
    "ERROR.TYPE(A1)",
    "LOG10(1)",
    "SUM  (1)",
    "=\"日本語\"&A1",
    "=A1\u{3000}+1",
    "=1+2 ",
    "  =1",
    "=A1:B2 C3",
    "=Ünïcode.A1",
    "=x.y.z",
    "=1.5E+3%",
    "=.5e-2",
    "=A1:A",
    "=A:1",
    "=$A$",
    "=A$$1",
    "=Err:",
    "=err:7",
    "=Err:5x",
    "=#N/A!",
    "=#ref!",
    "={1;2|3;4}",
    "=\"unterminated",
    "='unterminated",
    "=1<>2<=3>=4<5>6",
    "=TRUE()",
    "=_a.b_c(1)",
];

fn dialect_name(src: &str, d: Dialect) -> &'static str {
    if d.is_open_formula() || src.starts_with("of:=") || src.starts_with("oooc:=") {
        "open-formula"
    } else {
        "calc-a1"
    }
}

fn dialect_from(name: &str) -> Dialect {
    match name {
        "open-formula" => Dialect::open_formula(),
        _ => Dialect::calc_a1_en_us(),
    }
}

fn lex(src: &str, d: Dialect) -> Fixture {
    let tokens = tokenize(src, d)
        .into_iter()
        .map(|t| Tok {
            kind: t.kind.name().to_string(),
            text: src[t.span.clone()].to_string(),
            start: t.span.start,
            end: t.span.end,
        })
        .collect();
    Fixture {
        src: src.to_string(),
        dialect: dialect_name(src, d).to_string(),
        tokens,
    }
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tokens")
}

fn corpus() -> Vec<(String, Dialect, &'static str)> {
    let calc = Dialect::calc_a1_en_us();
    let groups: [(&str, &[&str], Dialect); 10] = [
        ("literals", LITERALS, calc),
        ("references", REFERENCES, calc),
        ("operators", OPERATORS, calc),
        ("functions", FUNCTIONS, calc),
        ("arrays", ARRAYS, calc),
        ("names", NAMES, calc),
        ("whitespace", WHITESPACE, calc),
        ("malformed", MALFORMED, calc),
        ("openformula", OPEN_FORMULA, Dialect::open_formula()),
        ("lexer", LEXER, calc),
    ];
    let mut out = Vec::new();
    for (group, srcs, d) in groups {
        for (i, s) in srcs.iter().enumerate() {
            out.push((format!("{group}-{:03}.json", i + 1), d, *s));
        }
    }
    out
}

#[test]
fn regenerate_when_requested() {
    if std::env::var_os("TALLY_REGEN_TOKEN_FIXTURES").is_none() {
        return;
    }
    let dir = dir();
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "json") {
            std::fs::remove_file(p).unwrap();
        }
    }
    for (file, d, src) in corpus() {
        let json = serde_json::to_string_pretty(&lex(src, d)).unwrap();
        std::fs::write(dir.join(file), json + "\n").unwrap();
    }
}

#[test]
fn lexer_matches_fixtures() {
    let mut n = 0;
    let mut entries: Vec<_> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap();
        let fx: Fixture = serde_json::from_str(&text).unwrap();
        let got = lex(&fx.src, dialect_from(&fx.dialect));
        assert_eq!(got, fx, "{}", path.display());
        for t in &fx.tokens {
            assert!(TokenKind::from_name(&t.kind).is_some(), "{}", t.kind);
        }
        n += 1;
    }
    assert!(n >= 150, "only {n} token fixtures");
}

#[test]
fn every_corpus_entry_has_a_fixture() {
    for (file, _, src) in corpus() {
        assert!(
            dir().join(&file).exists(),
            "missing fixture {file} for {src:?}"
        );
    }
}
