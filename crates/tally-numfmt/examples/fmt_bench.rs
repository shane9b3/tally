//! Formatting micro-benchmark: `cargo run --release -p tally-numfmt --example fmt_bench`.
//!
//! Formats a fixed set of values with the common Western codes (the hot
//! path of grid rendering) and prints nanoseconds per call, so changes to
//! the formatter can be compared before / after.

use std::hint::black_box;
use std::time::Instant;

use tally_numfmt::{format_value_with, parse_format, EN_US};
use tally_types::{DateEpoch, Value};

fn main() {
    let codes = [
        "General",
        "0.00",
        "#,##0.00",
        "0%",
        "0.00E+00",
        "# ?/?",
        "MM/DD/YYYY",
        "YYYY-MM-DD HH:MM:SS",
        "[$-411]GGGE\"年\"M\"月\"D\"日\"",
        "[NatNum1][$-411]#,##0",
    ];
    let values: Vec<Value> = (0..1000)
        .map(|i| Value::Number(40000.0 + f64::from(i) * 1.37))
        .collect();
    let rounds = 400;
    for code in codes {
        let ast = parse_format(code).expect("valid code");
        // Warm up.
        for v in &values {
            black_box(format_value_with(
                v,
                &ast,
                &EN_US,
                Some(12),
                DateEpoch::D1899,
            ));
        }
        let t = Instant::now();
        for _ in 0..rounds {
            for v in &values {
                black_box(format_value_with(
                    black_box(v),
                    &ast,
                    &EN_US,
                    Some(12),
                    DateEpoch::D1899,
                ));
            }
        }
        let ns = t.elapsed().as_nanos() as f64 / f64::from(rounds * 1000);
        println!("{code:<34} {ns:8.1} ns/call");
    }
}
