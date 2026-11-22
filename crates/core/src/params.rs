//! Parameter descriptions (for building UIs/CLIs) and validated parameter values.

use std::collections::BTreeMap;
use std::fmt::Display;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Value {
    /// Text form used to compare a value with the options of a `Visibility` condition.
    fn condition_text(&self) -> String {
        match self {
            Value::Bool(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::Float(v) => v.to_string(),
            Value::Text(v) => v.clone(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Text(_) => "text",
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(i64::from(v))
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

/// What a parameter accepts. A choice stores the option id as text;
/// labels come from localization keys, not from the core.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Bool { default: bool },
    Int { default: i64, min: Option<i64>, max: Option<i64> },
    Float { default: f64, min: Option<f64>, max: Option<f64> },
    Text { default: String },
    Choice { options: Vec<String>, default: String },
}

impl ParamKind {
    fn type_name(&self) -> &'static str {
        match self {
            ParamKind::Bool { .. } => "bool",
            ParamKind::Int { .. } => "int",
            ParamKind::Float { .. } => "float",
            ParamKind::Text { .. } | ParamKind::Choice { .. } => "text",
        }
    }
}

/// "Show this parameter only while parameter `param` has one of these values."
/// A hint for UIs; the tool still receives every parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Visibility {
    pub param: String,
    pub one_of: Vec<String>,
}

/// Description of one tool parameter. Every parameter has a default.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
    pub id: String,
    pub kind: ParamKind,
    /// All conditions must hold for the parameter to be shown (none = always shown).
    pub shown_when: Vec<Visibility>,
}

impl ParamSpec {
    pub fn new(id: impl Into<String>, kind: ParamKind) -> Self {
        Self { id: id.into(), kind, shown_when: Vec::new() }
    }

    /// Shows the parameter only while `param` (a choice, bool or number) has one
    /// of `values`, written as text. Can be chained: all conditions must hold.
    pub fn show_when(mut self, param: &str, values: &[&str]) -> Self {
        self.shown_when.push(Visibility {
            param: param.to_owned(),
            one_of: values.iter().map(|v| (*v).to_owned()).collect(),
        });
        self
    }

    /// Whether a UI should show this parameter; `current` returns the present
    /// value of another parameter. A condition on an unknown value is not met.
    pub fn is_shown(&self, current: impl Fn(&str) -> Option<Value>) -> bool {
        self.shown_when.iter().all(|c| {
            current(&c.param).is_some_and(|v| c.one_of.contains(&v.condition_text()))
        })
    }

    pub fn boolean(id: impl Into<String>, default: bool) -> Self {
        Self::new(id, ParamKind::Bool { default })
    }

    pub fn int(id: impl Into<String>, default: i64) -> Self {
        Self::new(id, ParamKind::Int { default, min: None, max: None })
    }

    pub fn int_range(id: impl Into<String>, default: i64, min: i64, max: i64) -> Self {
        Self::new(id, ParamKind::Int { default, min: Some(min), max: Some(max) })
    }

    pub fn float(id: impl Into<String>, default: f64) -> Self {
        Self::new(id, ParamKind::Float { default, min: None, max: None })
    }

    pub fn float_range(id: impl Into<String>, default: f64, min: f64, max: f64) -> Self {
        Self::new(id, ParamKind::Float { default, min: Some(min), max: Some(max) })
    }

    pub fn text(id: impl Into<String>, default: impl Into<String>) -> Self {
        Self::new(id, ParamKind::Text { default: default.into() })
    }

    pub fn choice(id: impl Into<String>, options: &[&str], default: &str) -> Self {
        Self::new(
            id,
            ParamKind::Choice {
                options: options.iter().map(|o| (*o).to_owned()).collect(),
                default: default.to_owned(),
            },
        )
    }

    pub fn default_value(&self) -> Value {
        match &self.kind {
            ParamKind::Bool { default } => Value::Bool(*default),
            ParamKind::Int { default, .. } => Value::Int(*default),
            ParamKind::Float { default, .. } => Value::Float(*default),
            ParamKind::Text { default } | ParamKind::Choice { default, .. } => {
                Value::Text(default.clone())
            }
        }
    }

    /// Checks that the spec itself is consistent (bounds, default, options).
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(Error::InvalidParam {
                name: String::new(),
                reason: "empty parameter id".to_owned(),
            });
        }
        let bounds_ok = match &self.kind {
            ParamKind::Int { min: Some(lo), max: Some(hi), .. } => lo <= hi,
            ParamKind::Float { min: Some(lo), max: Some(hi), .. } => lo <= hi,
            _ => true,
        };
        if !bounds_ok {
            return Err(self.invalid("minimum is greater than maximum"));
        }
        self.check(&self.default_value()).map(|_| ())
    }

    /// Validates a value against this spec and returns it normalized
    /// (an int given for a float parameter becomes a float).
    pub fn check(&self, value: &Value) -> Result<Value> {
        match &self.kind {
            ParamKind::Bool { .. } => match value {
                Value::Bool(v) => Ok(Value::Bool(*v)),
                other => Err(self.mismatch(other)),
            },
            ParamKind::Int { min, max, .. } => match value {
                Value::Int(v) => {
                    check_range(*v, *min, *max).map_err(|r| self.invalid(r))?;
                    Ok(Value::Int(*v))
                }
                other => Err(self.mismatch(other)),
            },
            ParamKind::Float { min, max, .. } => {
                let v = match value {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    other => return Err(self.mismatch(other)),
                };
                if !v.is_finite() {
                    return Err(self.invalid("must be a finite number"));
                }
                check_range(v, *min, *max).map_err(|r| self.invalid(r))?;
                Ok(Value::Float(v))
            }
            ParamKind::Text { .. } => match value {
                Value::Text(s) => Ok(Value::Text(s.clone())),
                other => Err(self.mismatch(other)),
            },
            ParamKind::Choice { options, .. } => match value {
                Value::Text(s) if options.contains(s) => Ok(Value::Text(s.clone())),
                Value::Text(_) => {
                    Err(self.invalid(format!("must be one of: {}", options.join(", "))))
                }
                other => Err(self.mismatch(other)),
            },
        }
    }

    fn invalid(&self, reason: impl Into<String>) -> Error {
        Error::InvalidParam { name: self.id.clone(), reason: reason.into() }
    }

    fn mismatch(&self, got: &Value) -> Error {
        self.invalid(format!("expected {}, got {}", self.kind.type_name(), got.type_name()))
    }
}

fn check_range<T: PartialOrd + Copy + Display>(
    v: T,
    min: Option<T>,
    max: Option<T>,
) -> std::result::Result<(), String> {
    match (min, max) {
        (Some(lo), _) if v < lo => Err(format!("{v} is below the minimum {lo}")),
        (_, Some(hi)) if v > hi => Err(format!("{v} is above the maximum {hi}")),
        _ => Ok(()),
    }
}

/// Parameter values by id. After `resolve` every spec'd id is present and valid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    values: BTreeMap<String, Value>,
}

impl Params {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, id: impl Into<String>, value: impl Into<Value>) -> Self {
        self.set(id, value);
        self
    }

    pub fn set(&mut self, id: impl Into<String>, value: impl Into<Value>) {
        self.values.insert(id.into(), value.into());
    }

    pub fn get(&self, id: &str) -> Option<&Value> {
        self.values.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn get_bool(&self, id: &str) -> Result<bool> {
        match self.require(id)? {
            Value::Bool(v) => Ok(*v),
            other => Err(type_error(id, "bool", other)),
        }
    }

    pub fn get_int(&self, id: &str) -> Result<i64> {
        match self.require(id)? {
            Value::Int(v) => Ok(*v),
            other => Err(type_error(id, "int", other)),
        }
    }

    pub fn get_float(&self, id: &str) -> Result<f64> {
        match self.require(id)? {
            Value::Float(v) => Ok(*v),
            Value::Int(v) => Ok(*v as f64),
            other => Err(type_error(id, "float", other)),
        }
    }

    pub fn get_text(&self, id: &str) -> Result<&str> {
        match self.require(id)? {
            Value::Text(v) => Ok(v),
            other => Err(type_error(id, "text", other)),
        }
    }

    fn require(&self, id: &str) -> Result<&Value> {
        self.values.get(id).ok_or_else(|| Error::MissingParam(id.to_owned()))
    }
}

fn type_error(id: &str, expected: &str, got: &Value) -> Error {
    Error::InvalidParam {
        name: id.to_owned(),
        reason: format!("expected {expected}, got {}", got.type_name()),
    }
}

/// Validates `given` against `specs`: rejects unknown ids, checks types and
/// ranges, fills defaults. The result is what a tool receives.
pub fn resolve(specs: &[ParamSpec], given: &Params) -> Result<Params> {
    for key in given.values.keys() {
        if !specs.iter().any(|s| &s.id == key) {
            return Err(Error::InvalidParam {
                name: key.clone(),
                reason: "unknown parameter".to_owned(),
            });
        }
    }
    let mut out = Params::new();
    for spec in specs {
        let value = match given.values.get(&spec.id) {
            Some(v) => spec.check(v)?,
            None => spec.default_value(),
        };
        out.values.insert(spec.id.clone(), value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ParamSpec> {
        vec![
            ParamSpec::int_range("n", 5, 1, 10),
            ParamSpec::float_range("ratio", 0.5, 0.0, 1.0),
            ParamSpec::boolean("flag", false),
            ParamSpec::text("label", "x"),
            ParamSpec::choice("mode", &["fast", "best"], "fast"),
        ]
    }

    #[test]
    fn defaults_are_filled() {
        let p = resolve(&specs(), &Params::new()).unwrap();
        assert_eq!(p.get_int("n").unwrap(), 5);
        assert_eq!(p.get_float("ratio").unwrap(), 0.5);
        assert!(!p.get_bool("flag").unwrap());
        assert_eq!(p.get_text("label").unwrap(), "x");
        assert_eq!(p.get_text("mode").unwrap(), "fast");
    }

    #[test]
    fn given_values_override_defaults() {
        let given = Params::new().with("n", 7).with("mode", "best").with("flag", true);
        let p = resolve(&specs(), &given).unwrap();
        assert_eq!(p.get_int("n").unwrap(), 7);
        assert_eq!(p.get_text("mode").unwrap(), "best");
        assert!(p.get_bool("flag").unwrap());
    }

    #[test]
    fn int_is_coerced_for_float_param() {
        let p = resolve(&specs(), &Params::new().with("ratio", 1)).unwrap();
        assert_eq!(p.get("ratio"), Some(&Value::Float(1.0)));
    }

    #[test]
    fn rejects_bad_input() {
        let bad = [
            Params::new().with("n", 0),
            Params::new().with("n", 11),
            Params::new().with("n", "five"),
            Params::new().with("ratio", f64::NAN),
            Params::new().with("ratio", 1.5),
            Params::new().with("mode", "slow"),
            Params::new().with("flag", 1),
            Params::new().with("nope", 1),
        ];
        for given in bad {
            assert!(
                matches!(resolve(&specs(), &given), Err(Error::InvalidParam { .. })),
                "accepted: {given:?}"
            );
        }
    }

    #[test]
    fn missing_and_mistyped_getters_fail() {
        let p = Params::new().with("n", 1);
        assert!(matches!(p.get_int("zzz"), Err(Error::MissingParam(_))));
        assert!(matches!(p.get_text("n"), Err(Error::InvalidParam { .. })));
    }

    #[test]
    fn inconsistent_specs_are_rejected() {
        assert!(ParamSpec::int_range("a", 5, 10, 1).validate().is_err());
        assert!(ParamSpec::int_range("a", 50, 1, 10).validate().is_err());
        assert!(ParamSpec::choice("a", &["x"], "y").validate().is_err());
        assert!(ParamSpec::choice("a", &[], "").validate().is_err());
        assert!(ParamSpec::text("", "").validate().is_err());
        assert!(ParamSpec::float_range("a", 0.5, 0.0, 1.0).validate().is_ok());
    }

    #[test]
    fn visibility_follows_other_parameters() {
        let spec = ParamSpec::int("w", 1).show_when("mode", &["a", "b"]).show_when("on", &["true"]);
        let with = |mode: &str, on: bool| {
            spec.is_shown(|id| match id {
                "mode" => Some(Value::from(mode)),
                "on" => Some(Value::Bool(on)),
                _ => None,
            })
        };
        assert!(with("a", true));
        assert!(with("b", true));
        assert!(!with("c", true));
        assert!(!with("a", false));
        assert!(!spec.is_shown(|_| None));
        assert!(ParamSpec::int("x", 1).is_shown(|_| None));
    }
}
