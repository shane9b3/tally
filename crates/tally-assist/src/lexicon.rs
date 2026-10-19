//! Synonym lexicon, tokenizer and fuzzy matching for the rule-based assist.

/// An aggregate intent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Agg {
    /// SUM
    Sum,
    /// AVERAGE
    Average,
    /// COUNT / COUNTA / COUNTIF
    Count,
    /// MAX
    Max,
    /// MIN
    Min,
    /// MEDIAN
    Median,
}

/// A comparison operator in a condition.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cmp {
    /// equals
    Eq,
    /// not equal
    Ne,
    /// greater than
    Gt,
    /// less than
    Lt,
    /// at least
    Ge,
    /// at most
    Le,
}

impl Cmp {
    /// Criteria prefix as written in a formula (`Eq` has none).
    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Eq => "",
            Cmp::Ne => "<>",
            Cmp::Gt => ">",
            Cmp::Lt => "<",
            Cmp::Ge => ">=",
            Cmp::Le => "<=",
        }
    }

    /// Words used in explanations.
    pub fn words(self) -> &'static str {
        match self {
            Cmp::Eq => "",
            Cmp::Ne => "not",
            Cmp::Gt => "over",
            Cmp::Lt => "under",
            Cmp::Ge => "at least",
            Cmp::Le => "at most",
        }
    }
}

/// One query word. `text` is lowercased and accent-folded; `orig` keeps the
/// user's spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Normalised text.
    pub text: String,
    /// Original spelling (punctuation trimmed).
    pub orig: String,
}

/// Folds common accented letters to ASCII and lowercases.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

fn is_op_char(c: char) -> bool {
    matches!(c, '<' | '>' | '=' | '!' | '≠' | '≥' | '≤')
}

fn canon_op(op: &str) -> String {
    match op {
        "≠" | "!=" | "<>" | "=!" => "<>".into(),
        "≥" | ">=" | "=>" => ">=".into(),
        "≤" | "<=" | "=<" => "<=".into(),
        "==" | "=" => "=".into(),
        other => other.into(),
    }
}

/// Splits a query into tokens: lowercased words, numbers and operator tokens.
/// Possessives are stripped and `n't` is expanded to `not`.
pub fn tokenize(query: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let q = query.replace('’', "'");
    for raw in q.split_whitespace() {
        for (is_op, piece) in split_ops(raw) {
            if is_op {
                let t = canon_op(&piece);
                if t.chars().all(|c| c == '!') {
                    continue;
                }
                out.push(Token {
                    text: t.clone(),
                    orig: t,
                });
                continue;
            }
            push_word(&mut out, &piece);
        }
    }
    out
}

/// Splits a raw word into runs of operator characters and other text.
fn split_ops(raw: &str) -> Vec<(bool, String)> {
    let mut pieces: Vec<(bool, String)> = Vec::new();
    for c in raw.chars() {
        let op = is_op_char(c);
        match pieces.last_mut() {
            Some((o, s)) if *o == op => s.push(c),
            _ => pieces.push((op, c.to_string())),
        }
    }
    pieces
}

fn push_word(out: &mut Vec<Token>, piece: &str) {
    let word = piece.trim_matches(|c: char| {
        matches!(
            c,
            '?' | '!' | '.' | ',' | ';' | ':' | '(' | ')' | '"' | '\''
        )
    });
    if word.is_empty() {
        return;
    }
    let lower = word.to_lowercase();
    let (mut w, negate) = match lower.strip_suffix("n't") {
        Some(stem) => (word[..stem.len().min(word.len())].to_string(), true),
        None => (word.to_string(), false),
    };
    if let Some(s) = w.strip_suffix("'s").or_else(|| w.strip_suffix("'S")) {
        w = s.to_string();
    }
    let w = w.trim_matches('\'').to_string();
    if w.is_empty() {
        return;
    }
    out.push(Token {
        text: fold(&w),
        orig: w,
    });
    if negate {
        out.push(Token {
            text: "not".into(),
            orig: "not".into(),
        });
    }
}

/// Crude English singularisation (plural and possessive stripping).
pub fn singular(w: &str) -> String {
    let n = w.chars().count();
    if n > 4 && w.ends_with("ies") {
        return format!("{}y", &w[..w.len() - 3]);
    }
    if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is") {
        return w[..w.len() - 1].to_string();
    }
    w.to_string()
}

/// Splits a cell value or header into folded, singularised word tokens.
pub fn value_words(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(singular)
        .collect()
}

/// Jaro similarity.
fn jaro(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut a_match = vec![false; a.len()];
    let mut b_match = vec![false; b.len()];
    let mut matches = 0usize;
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        for j in lo..hi {
            if !b_match[j] && b[j] == *ca {
                a_match[i] = true;
                b_match[j] = true;
                matches += 1;
                break;
            }
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut transpositions = 0usize;
    let mut j = 0usize;
    for (i, ca) in a.iter().enumerate() {
        if a_match[i] {
            while !b_match[j] {
                j += 1;
            }
            if *ca != b[j] {
                transpositions += 1;
            }
            j += 1;
        }
    }
    let m = matches as f64;
    (m / a.len() as f64 + m / b.len() as f64 + (m - (transpositions / 2) as f64) / m) / 3.0
}

/// Jaro-Winkler similarity in `0.0..=1.0` (prefix scale 0.1, prefix cap 4).
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let j = jaro(&ac, &bc);
    let prefix = ac
        .iter()
        .zip(bc.iter())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count();
    j + prefix as f64 * 0.1 * (1.0 - j)
}

/// Parses a number token: `10`, `1,000`, `$12.50`, `-3`.
pub fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim_start_matches('$');
    if s.is_empty() || !s.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let cleaned: String = s.chars().filter(|&c| c != ',').collect();
    if !cleaned
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
    {
        return None;
    }
    cleaned.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A matched aggregate keyword.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AggMatch {
    /// The intent.
    pub agg: Agg,
    /// First token index.
    pub start: usize,
    /// Number of tokens.
    pub len: usize,
    /// Matched by fuzzy spelling.
    pub fuzzy: bool,
    /// Weak keywords (`total`, `top`) lose to any strong keyword.
    pub weak: bool,
}

const MULTI: &[(&[&str], Agg)] = &[
    (&["add", "up"], Agg::Sum),
    (&["sum", "of"], Agg::Sum),
    (&["sum", "up"], Agg::Sum),
    (&["how", "many"], Agg::Count),
    (&["number", "of"], Agg::Count),
    (&["count", "of"], Agg::Count),
];

/// Single-word synonyms (singular form) with the weak flag.
const SINGLE: &[(&str, Agg, bool)] = &[
    ("total", Agg::Sum, true),
    ("sum", Agg::Sum, false),
    ("add", Agg::Sum, false),
    ("average", Agg::Average, false),
    ("mean", Agg::Average, false),
    ("avg", Agg::Average, false),
    ("count", Agg::Count, false),
    ("max", Agg::Max, false),
    ("maximum", Agg::Max, false),
    ("highest", Agg::Max, false),
    ("largest", Agg::Max, false),
    ("biggest", Agg::Max, false),
    ("top", Agg::Max, true),
    ("min", Agg::Min, false),
    ("minimum", Agg::Min, false),
    ("lowest", Agg::Min, false),
    ("smallest", Agg::Min, false),
    ("least", Agg::Min, false),
    ("median", Agg::Median, false),
];

/// Finds the aggregate keyword. `skip(i)` marks tokens that must not be
/// fuzzy-matched (for example those already used as a category value).
pub fn find_aggregate(tokens: &[Token], skip: &dyn Fn(usize) -> bool) -> Option<AggMatch> {
    let mut cands: Vec<AggMatch> = Vec::new();
    let sing: Vec<String> = tokens.iter().map(|t| singular(&t.text)).collect();
    let mut i = 0;
    while i < tokens.len() {
        let mut hit = false;
        for (words, agg) in MULTI {
            if i + words.len() <= tokens.len()
                && words
                    .iter()
                    .enumerate()
                    .all(|(k, w)| tokens[i + k].text == *w || sing[i + k] == *w)
            {
                cands.push(AggMatch {
                    agg: *agg,
                    start: i,
                    len: words.len(),
                    fuzzy: false,
                    weak: false,
                });
                i += words.len();
                hit = true;
                break;
            }
        }
        if hit {
            continue;
        }
        for (w, agg, weak) in SINGLE {
            if sing[i] == *w {
                // "at least" is a comparator, not MIN.
                if *w == "least" && i > 0 && tokens[i - 1].text == "at" {
                    continue;
                }
                cands.push(AggMatch {
                    agg: *agg,
                    start: i,
                    len: 1,
                    fuzzy: false,
                    weak: *weak,
                });
                break;
            }
        }
        i += 1;
    }
    if let Some(c) = cands.iter().find(|c| !c.weak).or_else(|| cands.first()) {
        return Some(*c);
    }
    // Fuzzy pass for misspellings.
    let mut best: Option<(f64, AggMatch)> = None;
    for (i, t) in tokens.iter().enumerate() {
        if skip(i)
            || t.text.chars().count() < 4
            || parse_number(&t.text).is_some()
            || SINGLE.iter().any(|(w, _, _)| *w == sing[i])
        {
            continue;
        }
        for (w, agg, weak) in SINGLE {
            if w.len() < 5 || t.text.chars().next() != w.chars().next() {
                continue;
            }
            let s = jaro_winkler(&sing[i], w);
            if s >= 0.88 && best.as_ref().is_none_or(|(b, _)| s > *b) {
                best = Some((
                    s,
                    AggMatch {
                        agg: *agg,
                        start: i,
                        len: 1,
                        fuzzy: true,
                        weak: *weak,
                    },
                ));
            }
        }
    }
    best.map(|(_, m)| m)
}

const COMPARATORS: &[(&[&str], Cmp)] = &[
    (&["greater", "than", "or", "equal", "to"], Cmp::Ge),
    (&["less", "than", "or", "equal", "to"], Cmp::Le),
    (&["not", "equal", "to"], Cmp::Ne),
    (&["no", "less", "than"], Cmp::Ge),
    (&["no", "more", "than"], Cmp::Le),
    (&["at", "least"], Cmp::Ge),
    (&["at", "most"], Cmp::Le),
    (&["greater", "than"], Cmp::Gt),
    (&["more", "than"], Cmp::Gt),
    (&["bigger", "than"], Cmp::Gt),
    (&["larger", "than"], Cmp::Gt),
    (&["higher", "than"], Cmp::Gt),
    (&["less", "than"], Cmp::Lt),
    (&["fewer", "than"], Cmp::Lt),
    (&["smaller", "than"], Cmp::Lt),
    (&["lower", "than"], Cmp::Lt),
    (&["equal", "to"], Cmp::Eq),
    (&["is", "not"], Cmp::Ne),
    (&["are", "not"], Cmp::Ne),
    (&["over"], Cmp::Gt),
    (&["above"], Cmp::Gt),
    (&["exceeding"], Cmp::Gt),
    (&["under"], Cmp::Lt),
    (&["below"], Cmp::Lt),
    (&["equals"], Cmp::Eq),
    (&["is"], Cmp::Eq),
    (&["are"], Cmp::Eq),
    (&[">="], Cmp::Ge),
    (&["<="], Cmp::Le),
    (&["<>"], Cmp::Ne),
    (&[">"], Cmp::Gt),
    (&["<"], Cmp::Lt),
    (&["="], Cmp::Eq),
];

/// Matches a comparator phrase at `i`, returning the operator and the number
/// of tokens consumed (the longest phrase wins).
pub fn parse_comparator(tokens: &[Token], i: usize) -> Option<(Cmp, usize)> {
    COMPARATORS.iter().find_map(|(words, cmp)| {
        (i + words.len() <= tokens.len()
            && words
                .iter()
                .enumerate()
                .all(|(k, w)| tokens[i + k].text == *w))
        .then_some((*cmp, words.len()))
    })
}

/// Month names (and three-letter abbreviations) as 1..=12.
pub fn month_number(w: &str) -> Option<u32> {
    const M: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    M.iter()
        .position(|m| *m == w || (w.len() == 3 && m.starts_with(w)))
        .and_then(|p| u32::try_from(p + 1).ok())
}

/// Words that never name a column or value.
pub fn is_stopword(w: &str) -> bool {
    matches!(
        w,
        "the"
            | "of"
            | "for"
            | "and"
            | "by"
            | "with"
            | "where"
            | "whose"
            | "in"
            | "is"
            | "are"
            | "a"
            | "an"
            | "all"
            | "my"
            | "me"
            | "show"
            | "what"
            | "whats"
            | "to"
            | "on"
            | "that"
            | "which"
            | "have"
            | "has"
            | "than"
            | "per"
            | "each"
            | "only"
            | "not"
            | "up"
            | "many"
            | "number"
            | "from"
            | "as"
            | "give"
            | "get"
            | "please"
            | "order"
            | "row"
            | "record"
            | "entry"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Token> {
        tokenize(s)
    }

    #[test]
    fn jaro_winkler_known_values() {
        assert!((jaro_winkler("martha", "marhta") - 0.9611).abs() < 1e-3);
        assert!((jaro_winkler("dixon", "dicksonx") - 0.8133).abs() < 1e-3);
        assert_eq!(jaro_winkler("same", "same"), 1.0);
        assert_eq!(jaro_winkler("abc", "xyz"), 0.0);
        assert!(jaro_winkler("avrage", "average") >= 0.88);
        assert!(jaro_winkler("totl", "total") >= 0.88);
    }

    #[test]
    fn tokenizer_handles_punctuation_and_operators() {
        let t: Vec<String> = toks("Total of Bob's orders?")
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert_eq!(t, ["total", "of", "bob", "orders"]);
        let t: Vec<String> = toks("qty>=20").into_iter().map(|t| t.text).collect();
        assert_eq!(t, ["qty", ">=", "20"]);
        let t: Vec<String> = toks("status isn't paid")
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert_eq!(t, ["status", "is", "not", "paid"]);
        let t: Vec<String> = toks("status ≠ Paid").into_iter().map(|t| t.text).collect();
        assert_eq!(t, ["status", "<>", "paid"]);
        assert_eq!(fold("Café"), "cafe");
    }

    #[test]
    fn singularisation() {
        assert_eq!(singular("orders"), "order");
        assert_eq!(singular("companies"), "company");
        assert_eq!(singular("status"), "status");
        assert_eq!(singular("less"), "less");
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_number("1,000"), Some(1000.0));
        assert_eq!(parse_number("$12.50"), Some(12.5));
        assert_eq!(parse_number("-3"), Some(-3.0));
        assert_eq!(parse_number("ten"), None);
        assert_eq!(parse_number("$"), None);
    }

    #[test]
    fn aggregates() {
        let none = |_: usize| false;
        let a = |q: &str| find_aggregate(&toks(q), &none).map(|m| m.agg);
        assert_eq!(a("total of unpaid"), Some(Agg::Sum));
        assert_eq!(a("add up the prices"), Some(Agg::Sum));
        assert_eq!(a("how many overdue"), Some(Agg::Count));
        assert_eq!(a("largest total"), Some(Agg::Max));
        assert_eq!(a("lowest price"), Some(Agg::Min));
        assert_eq!(a("mean price"), Some(Agg::Average));
        assert_eq!(a("median qty"), Some(Agg::Median));
        assert_eq!(a("avrage price"), Some(Agg::Average));
        assert_eq!(a("totl unpaid"), Some(Agg::Sum));
        assert_eq!(a("qty at least 20"), None);
        assert_eq!(a("hello"), None);
        assert_eq!(a("delete everything"), None);
    }

    #[test]
    fn comparators() {
        let t = toks("qty at least 20");
        assert_eq!(parse_comparator(&t, 1), Some((Cmp::Ge, 2)));
        let t = toks("price greater than or equal to 5");
        assert_eq!(parse_comparator(&t, 1), Some((Cmp::Ge, 5)));
        let t = toks("qty over 10");
        assert_eq!(parse_comparator(&t, 1), Some((Cmp::Gt, 1)));
        let t = toks("qty < 3");
        assert_eq!(parse_comparator(&t, 1), Some((Cmp::Lt, 1)));
        assert_eq!(parse_comparator(&t, 0), None);
    }

    #[test]
    fn months() {
        assert_eq!(month_number("october"), Some(10));
        assert_eq!(month_number("oct"), Some(10));
        assert_eq!(month_number("octopus"), None);
    }
}
