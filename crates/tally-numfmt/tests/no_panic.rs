//! Random format codes and inputs never panic (stand-in for the fuzz target).

use proptest::prelude::*;
use tally_numfmt::{
    format_value_with, parse_format, parse_number_unambiguous, print_format, recognize_input_with,
    Locale, RecognizeContext,
};
use tally_types::{DateEpoch, Value};

const CODE_CHARS: &str = "0#?.,%E+-e/@\"\\*_[]<>=;$ ()DMYHSAPNQWGmdyhs:1679RCOLBUTN~é€";
const INPUT_CHARS: &str = "0123456789.,%$-+()/: eEaApPmMTRUFLSOct'é";

fn from_set(set: &'static str, max: usize) -> impl Strategy<Value = String> {
    let chars: Vec<char> = set.chars().collect();
    proptest::collection::vec(proptest::sample::select(chars), 0..max)
        .prop_map(|v| v.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn formats_never_panic(code in from_set(CODE_CHARS, 24), x in proptest::num::f64::ANY, w in proptest::option::of(0u16..30)) {
        let l = Locale::en_us();
        if let Ok(ast) = parse_format(&code) {
            let printed = print_format(&ast);
            prop_assert!(parse_format(&printed).is_ok(), "{code:?} -> {printed:?}");
            for epoch in [DateEpoch::D1899, DateEpoch::D1904] {
                let _ = format_value_with(&Value::Number(x), &ast, &l, w, epoch);
                let _ = format_value_with(&Value::Number(x.fract() * 1e5), &ast, &l, w, epoch);
            }
            let _ = format_value_with(&Value::text("abc"), &ast, &l, w, DateEpoch::D1899);
        }
    }

    #[test]
    fn recognizer_never_panics(s in from_set(INPUT_CHARS, 20)) {
        let l = Locale::en_us();
        let ctx = RecognizeContext { epoch: DateEpoch::D1899, today: (2026, 10, 19) };
        let _ = recognize_input_with(&s, &l, &ctx);
        let _ = parse_number_unambiguous(&s);
    }
}
