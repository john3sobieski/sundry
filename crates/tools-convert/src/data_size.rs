//! `convert.data`: converts an amount of information between bits, bytes,
//! SI units (1 kB = 1000 B) and IEC units (1 KiB = 1024 B).

use sundry_core::{
    Category, Error, Input, InputKind, Item, Job, Output, ParamKind, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

/// Unit ids and their size in bits. The ids double as the labels in the UI.
const UNITS: [(&str, f64); 13] = [
    ("bit", 1.0),
    ("B", 8.0),
    ("kbit", 1e3),
    ("Mbit", 1e6),
    ("Gbit", 1e9),
    ("kB", 8e3),
    ("MB", 8e6),
    ("GB", 8e9),
    ("TB", 8e12),
    ("KiB", 8.0 * 1024.0),
    ("MiB", 8.0 * 1024.0 * 1024.0),
    ("GiB", 8.0 * 1024.0 * 1024.0 * 1024.0),
    ("TiB", 8.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
];

pub struct DataSize;

fn bits_per(unit: &str) -> Result<f64> {
    UNITS.iter().find(|(id, _)| *id == unit).map(|(_, bits)| *bits).ok_or_else(|| {
        Error::InvalidParam { name: "unit".to_owned(), reason: format!("unknown unit `{unit}`") }
    })
}

/// Rounds to 15 significant digits to hide binary noise (0.30000000000000004).
fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    let rounded: f64 = format!("{v:.14e}").parse().unwrap_or(v);
    rounded.to_string()
}

/// Converts `value` from one unit to another.
pub fn convert(value: f64, from: &str, to: &str) -> Result<f64> {
    let result = value * bits_per(from)? / bits_per(to)?;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(Error::InvalidInput("the result is too large".to_owned()))
    }
}

impl Tool for DataSize {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "convert.data",
            Category::Convert,
            "Data size",
            "Converts bits, bytes, SI units (kB = 1000 B) and IEC units (KiB = 1024 B).",
        )
        .with_input(InputKind::None)
    }

    fn params(&self) -> Vec<ParamSpec> {
        let ids: Vec<&str> = UNITS.iter().map(|(id, _)| *id).collect();
        vec![
            ParamSpec {
                id: "value".to_owned(),
                kind: ParamKind::Float { default: 1.0, min: Some(0.0), max: None },
            },
            ParamSpec::choice("from", &ids, "MB"),
            ParamSpec::choice("to", &ids, "MiB"),
        ]
    }

    fn run(&self, _: &Input, params: &Params, _: &Job) -> Result<Output> {
        let value = params.get_float("value")?;
        let from = params.get_text("from")?;
        let to = params.get_text("to")?;
        let result = convert(value, from, to)?;
        let text = format!("{} {from} = {} {to}", format_number(value), format_number(result));
        Ok(Output::single(Item::text(text)))
    }
}

#[cfg(test)]
mod tests {
    use sundry_core::Registry;

    use super::*;

    fn run(value: f64, from: &str, to: &str) -> Result<String> {
        let mut registry = Registry::new();
        registry.register(DataSize)?;
        let params = Params::new().with("value", value).with("from", from).with("to", to);
        let output = registry.run("convert.data", &Input::default(), &params, &Job::new())?;
        Ok(output.items[0].data.as_text()?.to_owned())
    }

    #[test]
    fn si_and_iec_differ() {
        assert_eq!(run(1.0, "kB", "B").unwrap(), "1 kB = 1000 B");
        assert_eq!(run(1.0, "KiB", "B").unwrap(), "1 KiB = 1024 B");
        assert_eq!(run(1.0, "MiB", "KiB").unwrap(), "1 MiB = 1024 KiB");
        assert_eq!(run(1.0, "TiB", "GB").unwrap(), "1 TiB = 1099.511627776 GB");
    }

    #[test]
    fn bits_and_bytes() {
        assert_eq!(run(1.0, "B", "bit").unwrap(), "1 B = 8 bit");
        assert_eq!(run(15.0, "kB", "Mbit").unwrap(), "15 kB = 0.12 Mbit");
        assert_eq!(run(16.0, "bit", "B").unwrap(), "16 bit = 2 B");
    }

    #[test]
    fn zero_and_fractions() {
        assert_eq!(run(0.0, "GB", "MB").unwrap(), "0 GB = 0 MB");
        assert_eq!(run(0.5, "KiB", "B").unwrap(), "0.5 KiB = 512 B");
        assert_eq!(run(1.0, "bit", "B").unwrap(), "1 bit = 0.125 B");
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(run(-1.0, "B", "bit").is_err());
        assert!(run(f64::NAN, "B", "bit").is_err());
        assert!(run(1.0, "parsec", "bit").is_err());
        assert!(convert(f64::MAX, "TiB", "bit").is_err());
    }

    #[test]
    fn spec_is_valid_and_units_are_unique() {
        let mut registry = Registry::new();
        registry.register(DataSize).unwrap();
        let mut ids: Vec<_> = UNITS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), UNITS.len());
    }
}
