//! Function Wizard metadata: a serializable view of the registry.
//!
//! [`FunctionMeta`] is what the wizard (⌘F2), the formula autocomplete and
//! the assist layer need about a function; the engine maps it onto the
//! API DTOs. See `docs/functions/wizard-metadata.md`.

use serde::{Deserialize, Serialize};

use crate::registry::{FunctionDef, Param, Registry};

/// The "Last used" list of a fresh profile, in display order (AOO's
/// default for the Function Wizard).
pub const LAST_USED_DEFAULT: [&str; 5] = ["SUM", "AVERAGE", "MIN", "MAX", "IF"];

/// One parameter of a function, for the wizard's argument fields.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamMeta {
    /// Display name (`"sum_range"`).
    pub name: String,
    /// [`crate::ParamKind::name`]: `"number"`, `"text"`, `"reference"`,
    /// `"numberSeq"`, `"lazy"`, …
    pub kind: String,
    /// The argument may be omitted.
    pub optional: bool,
    /// Part of the trailing repeating group (`SUM(number; ...)`).
    pub repeat: bool,
}

impl ParamMeta {
    fn from_param(p: &Param) -> Self {
        Self {
            name: p.name.to_owned(),
            kind: p.kind.name().to_owned(),
            optional: p.optional,
            repeat: p.repeat,
        }
    }
}

/// Everything the Function Wizard shows about one function.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionMeta {
    /// Canonical English name (`"SUMIF"`).
    pub name: String,
    /// AOO wizard category name (`"Mathematical"`, `"Date&Time"`, `"Add-in"`).
    pub category: String,
    /// Declared parameters; the trailing `repeat` ones form the repeating group.
    pub params: Vec<ParamMeta>,
    /// Minimum number of arguments.
    pub min_args: u8,
    /// Maximum number of arguments; 255 means unbounded.
    pub max_args: u8,
    /// One-line description.
    pub help: String,
    /// One example call in the en-US UI dialect (`=SUMIF(A1:A10;">5")`).
    pub example: String,
    /// Not in AOO 4.1 (IFERROR, IFNA, ISOWEEKNUM, NUMBERVALUE).
    pub extension: bool,
    /// Name stored in ODF (`"COM.MICROSOFT.IFERROR"`).
    pub odf_name: String,
    /// Name stored in OOXML when it differs from `name` (`"_xlfn.IFS"`).
    pub ooxml_name: Option<String>,
}

impl FunctionMeta {
    /// The metadata of one definition.
    pub fn from_def(def: &FunctionDef) -> Self {
        Self {
            name: def.name.to_owned(),
            category: def.category.aoo_name().to_owned(),
            params: def.params.iter().map(ParamMeta::from_param).collect(),
            min_args: def.min_args,
            max_args: def.max_args,
            help: def.help.to_owned(),
            example: def.example.to_owned(),
            extension: def.extension,
            odf_name: def.odf_name.to_owned(),
            ooxml_name: def.ooxml_name.map(str::to_owned),
        }
    }
}

impl Registry {
    /// The metadata of every registered function, in registration order.
    pub fn meta(&self) -> Vec<FunctionMeta> {
        self.iter()
            .map(|(_, d)| FunctionMeta::from_def(d))
            .collect()
    }
}

/// The metadata of every function in the global registry.
pub fn function_meta() -> Vec<FunctionMeta> {
    crate::registry().meta()
}

#[cfg(test)]
mod tests {
    use super::{function_meta, FunctionMeta, ParamMeta, LAST_USED_DEFAULT};
    use crate::registry;

    #[test]
    fn covers_the_registry() {
        let all = function_meta();
        assert_eq!(all.len(), registry().len());
        for m in &all {
            assert!(!m.name.is_empty());
            assert!(!m.help.is_empty(), "{} has no help text", m.name);
            assert!(
                m.example.starts_with(&format!("={}(", m.name)),
                "{}: example {:?} does not call the function",
                m.name,
                m.example
            );
            assert!(m.min_args <= m.max_args, "{}: arity", m.name);
        }
    }

    #[test]
    fn sumif_shape() {
        let m = function_meta()
            .into_iter()
            .find(|m| m.name == "SUMIF")
            .unwrap_or_else(|| panic!("SUMIF"));
        assert_eq!(m.category, "Mathematical");
        assert_eq!(m.odf_name, "SUMIF");
        assert_eq!(m.ooxml_name, None);
        assert!(!m.extension);
        assert_eq!((m.min_args, m.max_args), (2, 3));
        assert_eq!(
            m.params,
            vec![
                ParamMeta {
                    name: "range".into(),
                    kind: "reference".into(),
                    optional: false,
                    repeat: false
                },
                ParamMeta {
                    name: "criteria".into(),
                    kind: "criteria".into(),
                    optional: false,
                    repeat: false
                },
                ParamMeta {
                    name: "sum_range".into(),
                    kind: "reference".into(),
                    optional: true,
                    repeat: false
                },
            ]
        );
        let sum = function_meta()
            .into_iter()
            .find(|m| m.name == "SUM")
            .unwrap_or_else(|| panic!("SUM"));
        assert!(sum.params[0].repeat);
        assert_eq!(sum.max_args, 255);
        let ifna = function_meta()
            .into_iter()
            .find(|m| m.name == "IFNA")
            .unwrap_or_else(|| panic!("IFNA"));
        assert!(ifna.extension);
        assert_eq!(ifna.odf_name, "COM.MICROSOFT.IFNA");
        assert_eq!(ifna.ooxml_name.as_deref(), Some("_xlfn.IFNA"));
    }

    #[test]
    fn serde_camel_case_round_trip() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Doc {
            f: FunctionMeta,
        }
        let m = function_meta()
            .into_iter()
            .find(|m| m.name == "IFERROR")
            .unwrap_or_else(|| panic!("IFERROR"));
        let text = toml::to_string(&Doc { f: m.clone() }).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.contains("odfName = \"COM.MICROSOFT.IFERROR\""),
            "{text}"
        );
        assert!(text.contains("minArgs = 2"), "{text}");
        let back: Doc = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.f, m);
    }

    #[test]
    fn last_used_default_names_exist() {
        let r = registry();
        for name in LAST_USED_DEFAULT {
            assert!(r.lookup(name).is_some(), "{name}");
        }
    }
}
