//! UPPER, LOWER and PROPER.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `UPPER(text)`.
pub const UPPER: FunctionDef = FunctionDef::new("UPPER", Category::Text, PARAMS, upper)
    .returns(ReturnKind::Text)
    .help("Converts a text to upper case.");
/// `LOWER(text)`.
pub const LOWER: FunctionDef = FunctionDef::new("LOWER", Category::Text, PARAMS, lower)
    .returns(ReturnKind::Text)
    .help("Converts a text to lower case.");
/// `PROPER(text)`.
pub const PROPER: FunctionDef = FunctionDef::new("PROPER", Category::Text, PARAMS, proper)
    .returns(ReturnKind::Text)
    .help("Capitalizes the first letter of each word.");

fn upper(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::text(a.text(0)?.to_uppercase())))
}

fn lower(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::text(a.text(0)?.to_lowercase())))
}

/// A letter that follows a non-letter is capitalized, every other letter
/// is lower-cased (`"2nd"` becomes `"2Nd"`, as in AOO and Excel).
fn proper(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let mut out = String::with_capacity(s.len());
        let mut prev_letter = false;
        for ch in s.chars() {
            if ch.is_alphabetic() {
                if prev_letter {
                    out.extend(ch.to_lowercase());
                } else {
                    out.extend(ch.to_uppercase());
                }
                prev_letter = true;
            } else {
                out.push(ch);
                prev_letter = false;
            }
        }
        Ok(Value::text(out))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(UPPER);
    r.register(LOWER);
    r.register(PROPER);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn t(name: &str, v: EvalValue) -> Value {
        call(&mut MockEvalContext::new(), name, vec![v])
    }

    #[test]
    fn upper_lower() {
        assert_eq!(t("UPPER", EvalValue::text("aBc é")), Value::text("ABC É"));
        assert_eq!(t("LOWER", EvalValue::text("aBc É")), Value::text("abc é"));
        assert_eq!(t("UPPER", EvalValue::number(1.5)), Value::text("1.5"));
        assert_eq!(t("LOWER", EvalValue::bool(true)), Value::text("true"));
        assert_eq!(
            t("UPPER", EvalValue::error(ErrorKind::Div0)),
            Value::Error(ErrorKind::Div0)
        );
    }

    #[test]
    fn proper() {
        let p = |s| t("PROPER", EvalValue::text(s));
        assert_eq!(p("hello wORLD"), Value::text("Hello World"));
        assert_eq!(p("o'neil-smith"), Value::text("O'Neil-Smith"));
        assert_eq!(p("2nd place"), Value::text("2Nd Place"));
        assert_eq!(p(""), Value::text(""));
        assert_eq!(p("élan vital"), Value::text("Élan Vital"));
    }
}
