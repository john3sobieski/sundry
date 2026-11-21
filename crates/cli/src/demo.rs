//! A sample tool for trying the CLI and for tests. Registered only with
//! the `demo` feature; remove it once real tools exist.

use sundry_core::{Category, Input, Item, Job, Output, ParamSpec, Params, Result, Tool, ToolInfo};

pub struct Repeat;

impl Tool for Repeat {
    fn info(&self) -> ToolInfo {
        ToolInfo::new("text.repeat", Category::Text, "Repeat text", "Repeats the input text.")
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec::int_range("times", 2, 1, 5),
            ParamSpec::text("separator", ""),
            ParamSpec::boolean("upper", false),
        ]
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let text = input.first()?.data.as_text()?;
        let times = params.get_int("times")?;
        let mut parts = Vec::new();
        for i in 0..times {
            job.check_cancelled()?;
            parts.push(text);
            job.set_progress((i + 1) as f32 / times as f32);
        }
        let mut result = parts.join(params.get_text("separator")?);
        if params.get_bool("upper")? {
            result = result.to_uppercase();
        }
        Ok(Output::single(Item::text(result)))
    }
}
