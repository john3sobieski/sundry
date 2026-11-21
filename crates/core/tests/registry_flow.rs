//! End-to-end check of the public API with a sample tool.

use sundry_core::{
    Category, Error, Input, Item, Job, Output, ParamSpec, Params, Registry, Result, Tool, ToolInfo,
};

struct Repeat;

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
            parts.push(text.to_owned());
            job.set_progress((i + 1) as f32 / times as f32);
        }
        let mut result = parts.join(params.get_text("separator")?);
        if params.get_bool("upper")? {
            result = result.to_uppercase();
        }
        Ok(Output::single(Item::text(result)))
    }
}

struct Dummy(&'static str, Vec<ParamSpec>);

impl Tool for Dummy {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(self.0, Category::Dev, "Dummy", "Does nothing.")
    }

    fn params(&self) -> Vec<ParamSpec> {
        self.1.clone()
    }

    fn run(&self, _: &Input, _: &Params, _: &Job) -> Result<Output> {
        Ok(Output::default())
    }
}

fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(Repeat).unwrap();
    r
}

fn text_of(out: &Output) -> &str {
    out.items[0].data.as_text().unwrap()
}

#[test]
fn runs_with_defaults_and_overrides() {
    let r = registry();
    let input = Input::single(Item::text("ab"));
    let job = Job::new();

    let out = r.run("text.repeat", &input, &Params::new(), &job).unwrap();
    assert_eq!(text_of(&out), "abab");
    assert_eq!(job.progress(), 1.0);

    let params = Params::new().with("times", 3).with("separator", "-").with("upper", true);
    let out = r.run("text.repeat", &input, &params, &Job::new()).unwrap();
    assert_eq!(text_of(&out), "AB-AB-AB");
}

#[test]
fn invalid_params_never_reach_the_tool() {
    let r = registry();
    let input = Input::single(Item::text("ab"));
    for params in [Params::new().with("times", 99), Params::new().with("bogus", 1)] {
        let res = r.run("text.repeat", &input, &params, &Job::new());
        assert!(matches!(res, Err(Error::InvalidParam { .. })));
    }
}

#[test]
fn unknown_tool_and_bad_input() {
    let r = registry();
    let job = Job::new();
    let res = r.run("nope", &Input::default(), &Params::new(), &job);
    assert!(matches!(res, Err(Error::ToolNotFound(_))));
    let res = r.run("text.repeat", &Input::default(), &Params::new(), &job);
    assert!(matches!(res, Err(Error::InvalidInput(_))));
}

#[test]
fn cancelled_job_does_not_run() {
    let r = registry();
    let job = Job::new();
    job.cancel();
    let input = Input::single(Item::text("ab"));
    let res = r.run("text.repeat", &input, &Params::new(), &job);
    assert!(matches!(res, Err(Error::Cancelled)));
}

#[test]
fn registration_rules() {
    let mut r = registry();
    assert!(matches!(r.register(Repeat), Err(Error::DuplicateTool(_))));
    assert!(matches!(r.register(Dummy("Bad Id", vec![])), Err(Error::InvalidTool { .. })));
    let dup = vec![ParamSpec::boolean("a", false), ParamSpec::boolean("a", true)];
    assert!(matches!(r.register(Dummy("dup.params", dup)), Err(Error::InvalidTool { .. })));
    let broken = vec![ParamSpec::int_range("n", 50, 1, 10)];
    assert!(matches!(r.register(Dummy("broken.spec", broken)), Err(Error::InvalidTool { .. })));
    assert_eq!(r.len(), 1);
}

#[test]
fn listing_and_search() {
    let mut r = registry();
    r.register(Dummy("dev.noop", vec![])).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r.by_category(Category::Text).len(), 1);
    assert_eq!(r.by_category(Category::Image).len(), 0);
    assert_eq!(r.search("REPEAT").len(), 1);
    assert_eq!(r.search("nothing like this").len(), 0);
    assert_eq!(r.search("  ").len(), 2);
    let ids: Vec<_> = r.iter().map(|e| e.info().id.as_str()).collect();
    assert_eq!(ids, ["dev.noop", "text.repeat"]);
    assert_eq!(r.get("text.repeat").unwrap().params().len(), 3);
}

#[test]
fn tool_handle_can_run_on_another_thread() {
    let r = registry();
    let tool = r.get("text.repeat").unwrap().tool();
    let specs = r.get("text.repeat").unwrap().params().to_vec();
    let handle = std::thread::spawn(move || {
        let params = sundry_core::params::resolve(&specs, &Params::new()).unwrap();
        tool.run(&Input::single(Item::text("x")), &params, &Job::new())
    });
    let out = handle.join().unwrap().unwrap();
    assert_eq!(text_of(&out), "xx");
}
