//! `print_format(parse_format(code)) == code` for canonical codes.

use tally_numfmt::{parse_format, print_format};

const CODES: &[&str] = &[
    "General",
    "Standard",
    "0",
    "0.00",
    "#,##0",
    "#,##0.00",
    "0%",
    "0.00%",
    "[$$-409]#,##0.00",
    "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00",
    "$#,##0.00;[RED]-$#,##0.00",
    "[$€-407]#,##0.00",
    "[$USD] #,##0.00",
    "[$-409]0.00",
    "0.00E+00",
    "0.00E-00",
    "##0.0E+0",
    "0.00e+00",
    "@",
    "\"Name: \"@",
    "MM/DD/YYYY",
    "YYYY-MM-DD",
    "DD MMM YYYY",
    "HH:MM",
    "HH:MM:SS",
    "HH:MM AM/PM",
    "H:MM A/P",
    "[HH]:MM",
    "[H]:MM:SS",
    "[MM]:SS",
    "[SS]",
    "HH:MM:SS.000",
    "MM/DD/YYYY HH:MM",
    "DDDD, MMMM D, YYYY",
    "NNN",
    "NNNND",
    "Q QQ WW",
    "G GG GGG E EE",
    "MMMMM",
    "# ?/?",
    "# ??/??",
    "# ??/16",
    "?/?",
    "#,##0,",
    "0.0,,\"M\"",
    "0;(0)",
    "0;-0;\"zero\"",
    "0.00;-0.00;0.00;@",
    "[>=100]\"big\";[<0]\"neg\";\"small\"",
    "[<-1.5]\"low\";0.0",
    "[<>0]0;\"z\"",
    "[<=0]\"np\";0",
    "[=1]\"one\";0",
    "[RED][>0]0;[BLUE][<0]0;[GREEN]0",
    "[>0][RED]0",
    "[COLOR5]0",
    "[COLOR64]0",
    "[NatNum1]0",
    "[DBNum1]0",
    "[~buddhist]YYYY",
    "_(0_)",
    "0*-",
    "**0",
    "\\$0",
    "000-00-0000",
    "(000) 000-0000",
    "0\" units\"",
    "#.##",
    "?.??",
    "0;;",
    ";;;",
    "",
];

#[test]
fn print_round_trip() {
    let mut failures = Vec::new();
    for code in CODES {
        let ast = parse_format(code).unwrap();
        let printed = print_format(&ast);
        let want = if code.is_empty() { "General" } else { code };
        if printed != want {
            failures.push(format!("{code:?} printed as {printed:?}"));
        }
        // Printing is a fixed point.
        let again = parse_format(&printed).unwrap();
        if again != ast && !code.is_empty() {
            failures.push(format!("{code:?} reparses differently"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(CODES.len() >= 40);
}

#[test]
fn canonicalises_case() {
    let cases = [
        ("yyyy-mm-dd", "YYYY-MM-DD"),
        ("[red]0", "[RED]0"),
        ("[Color5]0", "[COLOR5]0"),
        ("general", "General"),
        ("[h]:mm", "[H]:MM"),
        ("[GRAY]0", "[GREY]0"),
        ("[natnum1]0", "[NatNum1]0"),
        ("h:mm am/pm", "H:MM am/pm"),
    ];
    for (code, want) in cases {
        assert_eq!(print_format(&parse_format(code).unwrap()), want, "{code}");
    }
}
