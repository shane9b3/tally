//! Typed syntax tree of an OOo number format code.
//!
//! A [`FormatAst`] keeps every token of the source code in order (so
//! [`crate::print_format`] can reproduce it) plus a per-section analysis
//! ([`SectionKind`]) that the formatter uses.

/// A parsed format code: 1 to 4 `;`-separated sections.
#[derive(Clone, PartialEq, Debug)]
pub struct FormatAst {
    /// The sections, in source order. Never empty.
    pub sections: Vec<Section>,
}

/// One `;`-separated section of a format code.
#[derive(Clone, PartialEq, Debug)]
pub struct Section {
    /// The section's `[<0]`-style condition, if any.
    pub condition: Option<Condition>,
    /// The section's colour, if any.
    pub color: Option<Color>,
    /// All tokens in source order, including the bracket modifiers.
    pub tokens: Vec<Token>,
    /// What the section formats and how.
    pub kind: SectionKind,
}

/// Comparison operator of a section condition.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CompareOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `=`
    Eq,
    /// `<>`
    Ne,
}

impl CompareOp {
    /// The operator's source text.
    pub fn as_str(self) -> &'static str {
        match self {
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
            CompareOp::Eq => "=",
            CompareOp::Ne => "<>",
        }
    }

    /// Evaluates `x op limit`.
    pub fn test(self, x: f64, limit: f64) -> bool {
        match self {
            CompareOp::Lt => x < limit,
            CompareOp::Le => x <= limit,
            CompareOp::Gt => x > limit,
            CompareOp::Ge => x >= limit,
            CompareOp::Eq => x == limit,
            CompareOp::Ne => x != limit,
        }
    }
}

/// A section condition such as `[>=100]`.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Condition {
    /// The comparison.
    pub op: CompareOp,
    /// The right-hand side.
    pub value: f64,
}

/// A named colour keyword.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum NamedColor {
    /// `[BLACK]` 0x000000
    Black,
    /// `[BLUE]` 0x0000FF
    Blue,
    /// `[GREEN]` 0x00FF00
    Green,
    /// `[CYAN]` 0x00FFFF
    Cyan,
    /// `[RED]` 0xFF0000
    Red,
    /// `[MAGENTA]` 0xFF00FF
    Magenta,
    /// `[BROWN]` 0x808000
    Brown,
    /// `[GREY]` / `[GRAY]` 0x808080
    Grey,
    /// `[YELLOW]` 0xFFFF00
    Yellow,
    /// `[WHITE]` 0xFFFFFF
    White,
}

impl NamedColor {
    /// All named colours.
    pub const ALL: [NamedColor; 10] = [
        NamedColor::Black,
        NamedColor::Blue,
        NamedColor::Green,
        NamedColor::Cyan,
        NamedColor::Red,
        NamedColor::Magenta,
        NamedColor::Brown,
        NamedColor::Grey,
        NamedColor::Yellow,
        NamedColor::White,
    ];

    /// The canonical (upper-case English) keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            NamedColor::Black => "BLACK",
            NamedColor::Blue => "BLUE",
            NamedColor::Green => "GREEN",
            NamedColor::Cyan => "CYAN",
            NamedColor::Red => "RED",
            NamedColor::Magenta => "MAGENTA",
            NamedColor::Brown => "BROWN",
            NamedColor::Grey => "GREY",
            NamedColor::Yellow => "YELLOW",
            NamedColor::White => "WHITE",
        }
    }

    /// The colour as `0xRRGGBB`. These are the OOo standard colours
    /// (`COL_LIGHTRED` for RED etc., see `zforscan.cxx`).
    pub fn rgb(self) -> u32 {
        match self {
            NamedColor::Black => 0x000000,
            NamedColor::Blue => 0x0000FF,
            NamedColor::Green => 0x00FF00,
            NamedColor::Cyan => 0x00FFFF,
            NamedColor::Red => 0xFF0000,
            NamedColor::Magenta => 0xFF00FF,
            NamedColor::Brown => 0x808000,
            NamedColor::Grey => 0x808080,
            NamedColor::Yellow => 0xFFFF00,
            NamedColor::White => 0xFFFFFF,
        }
    }
}

/// The Excel default 56-colour palette used for `[COLORn]`.
///
/// `[COLOR1]`..`[COLOR56]` index this table; `[COLOR57]`..`[COLOR64]`
/// wrap around to `[COLOR1]`..`[COLOR8]`.
pub const COLOR_PALETTE: [u32; 56] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, // 1-8
    0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, // 9-16
    0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, // 17-24
    0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, // 25-32
    0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, // 33-40
    0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, // 41-48
    0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333, // 49-56
];

/// A section colour.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Color {
    /// A colour keyword such as `[RED]`.
    Named(NamedColor),
    /// `[COLORn]`, n in 1..=64.
    Index(u8),
}

impl Color {
    /// The colour as `0xRRGGBB`.
    pub fn rgb(self) -> u32 {
        match self {
            Color::Named(c) => c.rgb(),
            Color::Index(n) => {
                let i = (usize::from(n.max(1)) - 1) % COLOR_PALETTE.len();
                COLOR_PALETTE[i]
            }
        }
    }
}

/// A digit placeholder.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Digit {
    /// `0`: always shows a digit.
    Zero,
    /// `#`: shows only significant digits.
    Hash,
    /// `?`: like `#` but pads with a space.
    Question,
}

impl Digit {
    /// The placeholder character.
    pub fn as_char(self) -> char {
        match self {
            Digit::Zero => '0',
            Digit::Hash => '#',
            Digit::Question => '?',
        }
    }
}

/// Unit of an elapsed-time bracket (`[HH]`, `[MM]`, `[SS]`).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ElapsedUnit {
    /// `[H]`, `[HH]`: total hours.
    Hours,
    /// `[M]`, `[MM]`: total minutes.
    Minutes,
    /// `[S]`, `[SS]`: total seconds.
    Seconds,
}

/// Which keyword spelled the General format.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum GeneralKeyword {
    /// `General` (English).
    General,
    /// `Standard` (German / OOo internal).
    Standard,
    /// `BOOLEAN`: AOO's boolean format, TRUE / FALSE for numbers.
    Boolean,
}

/// A date or time keyword. Counts are the number of repeated letters.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum DateTimeToken {
    /// `D` (1) / `DD` (2): day of month.
    Day(u8),
    /// `DDD` (3): abbreviated weekday, `DDDD` (4+): full weekday.
    DayName(u8),
    /// `NN` (2) abbreviated, `NNN` (3) full, `NNNN` (4) full weekday plus
    /// the locale's separator.
    AooDayName(u8),
    /// `AAA` (3) / `AAAA` (4): abbreviated / full weekday (Japanese Excel).
    AaaDayName(u8),
    /// `M`/`MM` month number, `MMM` short name, `MMMM` full name, `MMMMM`
    /// first letter.
    Month(u8),
    /// `M`/`MM` meaning minutes (after an hour or before a second keyword).
    Minute(u8),
    /// `YY` two-digit year (also `Y`), `YYYY` four-digit year (also `YYY`).
    Year(u8),
    /// `H` / `HH`.
    Hour(u8),
    /// `S` / `SS`.
    Second(u8),
    /// `AM/PM` in its original spelling.
    AmPm(String),
    /// `A/P` in its original spelling.
    AP(String),
    /// `Q` (1) / `QQ` (2): quarter.
    Quarter(u8),
    /// `WW`: calendar week.
    Week(u8),
    /// `G` / `GG` / `GGG`: era.
    Era(u8),
    /// `E` / `EE`: year in era.
    EraYear(u8),
    /// `[H]`, `[MM]`, `[SS]` … elapsed time; count of letters.
    Elapsed(ElapsedUnit, u8),
}

/// One token of a section.
#[derive(Clone, PartialEq, Debug)]
pub enum Token {
    /// `0`, `#` or `?`.
    Digit(Digit),
    /// `.`: decimal separator (or the fractional-seconds point in times).
    DecimalPoint,
    /// `,` acting as thousands grouping or scaling (see [`NumberLayout`]).
    /// Commas in other positions are turned into [`Token::Literal`].
    Comma,
    /// `%`.
    Percent,
    /// `E+`, `E-`, `e+`, `e-`.
    Exponent {
        /// Written in lower case (`e+`).
        lower: bool,
        /// `E+` (always show the sign) rather than `E-`.
        plus: bool,
    },
    /// `/` acting as a fraction bar. Date separators are literals.
    Slash,
    /// `@`: the text value.
    At,
    /// `"text"`.
    Quoted(String),
    /// `\c`.
    Escaped(char),
    /// A bare literal character such as `$`, `-`, space, or a digit `1`-`9`.
    Literal(char),
    /// `*c`: fill the cell with `c`.
    Fill(char),
    /// `_c`: a space the width of `c`.
    Skip(char),
    /// A date or time keyword.
    DateTime(DateTimeToken),
    /// `General` / `Standard`.
    General(GeneralKeyword),
    /// `[RED]`, `[COLOR5]` …
    Color(Color),
    /// `[<0]` …
    Condition(Condition),
    /// `[$sym-LCID]`: shows `symbol`; the locale id is kept for printing.
    Currency {
        /// The symbol text (may be empty, as in `[$-409]`).
        symbol: String,
        /// The hex locale id after `-`, if any.
        locale: Option<String>,
    },
    /// `[NatNumN]`: parsed and ignored.
    NatNum(u8),
    /// `[DBNumN]`: parsed and ignored.
    DbNum(u8),
    /// `[~calendar]`: parsed and ignored.
    Calendar(String),
}

/// Exponent part of a scientific format.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ExponentLayout {
    /// Token index of the [`Token::Exponent`].
    pub index: usize,
    /// Minimum exponent digits (the placeholders after `E+`, at least 1).
    pub digits: u8,
    /// `E+` (true) or `E-` (false).
    pub plus: bool,
}

/// Fraction part of a fraction format such as `# ??/16`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FractionLayout {
    /// Token range of the integer part (`None` for improper fractions such
    /// as `?/?`).
    pub int_range: Option<(usize, usize)>,
    /// Token range of the numerator placeholders.
    pub num_range: (usize, usize),
    /// Token index of the [`Token::Slash`].
    pub slash: usize,
    /// Token range of the denominator (placeholders or fixed digits).
    pub den_range: (usize, usize),
    /// Number of denominator placeholders (max denominator `10^n - 1`).
    pub den_digits: u8,
    /// A fixed denominator such as 16 in `# ?/16`.
    pub fixed_den: Option<u32>,
}

/// Analysis of a number section.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct NumberLayout {
    /// Digit placeholders before the decimal point.
    pub int_digits: u16,
    /// `0` placeholders before the decimal point.
    pub min_int: u16,
    /// Digit placeholders after the decimal point.
    pub frac_digits: u16,
    /// `0` placeholders after the decimal point.
    pub min_frac: u16,
    /// Thousands grouping is on.
    pub grouping: bool,
    /// Number of scaling commas (each divides by 1000).
    pub scale_commas: u8,
    /// Number of `%` signs (each multiplies by 100).
    pub percent: u8,
    /// Token index of the decimal point, if any.
    pub decimal_index: Option<usize>,
    /// Integer region: tokens `[0, int_end)`.
    pub int_end: usize,
    /// Fraction region: tokens `(decimal_index, frac_end)`.
    pub frac_end: usize,
    /// Scientific notation.
    pub exponent: Option<ExponentLayout>,
    /// Fraction notation.
    pub fraction: Option<FractionLayout>,
}

/// Analysis of a date/time section.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct DateLayout {
    /// Contains `AM/PM` or `A/P` (12-hour clock).
    pub twelve_hour: bool,
    /// Digits of fractional seconds (`SS.000` → 3).
    pub frac_sec_digits: u8,
    /// Contains an elapsed-time bracket.
    pub elapsed: Option<ElapsedUnit>,
    /// Contains a date part (day, month, year, weekday, quarter, week, era).
    pub has_date: bool,
    /// Contains a time part.
    pub has_time: bool,
}

/// What a section formats.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum SectionKind {
    /// A number section (also literal-only and empty sections).
    Number(NumberLayout),
    /// A date and/or time section.
    Date(DateLayout),
    /// A text section (contains `@`, or is the 4th section).
    Text,
    /// Contains the `General` keyword.
    General,
}

impl FormatAst {
    /// Whether this is the plain `General` format, also with a language
    /// tag (`[$-407]General`: General in that language, as an ODS number
    /// style with only `number:language` gives).
    pub fn is_general(&self) -> bool {
        self.sections.len() == 1
            && matches!(self.sections[0].kind, SectionKind::General)
            && self.sections[0].tokens.iter().any(|t| {
                matches!(
                    t,
                    Token::General(GeneralKeyword::General | GeneralKeyword::Standard)
                )
            })
            && self.sections[0].tokens.iter().all(|t| match t {
                Token::General(k) => *k != GeneralKeyword::Boolean,
                Token::Currency { symbol, .. } => symbol.is_empty(),
                _ => false,
            })
    }

    /// Whether the first section is a date or time format.
    pub fn is_date(&self) -> bool {
        matches!(self.sections[0].kind, SectionKind::Date(_))
    }
}

/// Whether a code is General, also with a leading language tag
/// (`[$-407]General`), without parsing it.
pub fn is_general_code(code: &str) -> bool {
    let mut rest = code.trim();
    while let Some(r) = rest.strip_prefix("[$-") {
        match r.find(']') {
            Some(end) if r[..end].chars().all(|c| c.is_ascii_hexdigit()) => {
                rest = r[end + 1..].trim_start();
            }
            _ => return false,
        }
    }
    rest.is_empty() || rest.eq_ignore_ascii_case("General") || rest.eq_ignore_ascii_case("Standard")
}
