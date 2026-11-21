//! End-to-end tests of the CLI: arguments in, stdout/stderr/exit code and files out.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use sundry_cli::{Io, run};
use sundry_core::{
    Category, Input, Item, Job, Output, ParamSpec, Params, Registry, Result, Tool, ToolInfo,
};

/// A test tool that reads its input, so input handling can be tested.
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

    fn run(&self, input: &Input, params: &Params, _: &Job) -> Result<Output> {
        let text = input.first()?.data.as_text()?;
        let times = params.get_int("times")?;
        let mut result =
            vec![text; usize::try_from(times).unwrap()].join(params.get_text("separator")?);
        if params.get_bool("upper")? {
            result = result.to_uppercase();
        }
        Ok(Output::single(Item::text(result)))
    }
}

/// Returns fixed items, for testing how results are written.
struct Emit(&'static str, Vec<Item>);

impl Tool for Emit {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(self.0, Category::Dev, "Emit", "Returns fixed items.")
    }

    fn run(&self, _: &Input, _: &Params, _: &Job) -> Result<Output> {
        Ok(Output { items: self.1.clone() })
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sundry-cli-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Outcome {
    code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

impl Outcome {
    fn out(&self) -> &str {
        std::str::from_utf8(&self.stdout).unwrap()
    }
}

fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(Repeat).unwrap();
    r.register(Emit("dev.text", vec![Item::text("hello")])).unwrap();
    r.register(Emit("dev.binary", vec![Item::bytes(vec![0xff, 0xfe, 0x00])])).unwrap();
    let named = vec![Item::text("one").named("../one.txt"), Item::text("two").named("two.txt")];
    r.register(Emit("dev.named", named)).unwrap();
    r
}

fn cli_with(registry: &Registry, args: &[&str], stdin: &[u8]) -> Outcome {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let mut input = stdin;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = {
        let mut io = Io { stdin: &mut input, stdout: &mut stdout, stderr: &mut stderr };
        run(registry, &args, &mut io)
    };
    Outcome { code, stdout, stderr: String::from_utf8(stderr).unwrap() }
}

fn cli(args: &[&str]) -> Outcome {
    cli_with(&registry(), args, b"")
}

#[test]
fn help_and_version() {
    let out = cli(&["--help"]);
    assert_eq!(out.code, 0);
    assert!(out.out().contains("Usage:"));
    assert_eq!(cli(&[]).code, 0);
    let out = cli(&["--version"]);
    assert_eq!(out.code, 0);
    assert!(out.out().starts_with("sundry "));
}

#[test]
fn list_shows_all_tools() {
    let out = cli(&["list"]);
    assert_eq!(out.code, 0);
    assert!(out.out().contains("text.repeat"));
    assert!(out.out().contains("Repeat text"));
    assert!(out.out().contains("dev.binary"));
}

#[test]
fn list_filters_by_category_and_query() {
    let out = cli(&["list", "-c", "text"]);
    assert!(out.out().contains("text.repeat"));
    assert!(!out.out().contains("dev.binary"));
    assert!(cli(&["list", "-c", "image"]).out().contains("No tools match."));
    assert!(cli(&["list", "REPEAT"]).out().contains("text.repeat"));
    assert!(cli(&["list", "zzz"]).out().contains("No tools match."));
    assert_eq!(cli(&["list", "-c", "bogus"]).code, 2);
}

#[test]
fn list_with_empty_registry() {
    let out = cli_with(&Registry::new(), &["list"], b"");
    assert_eq!(out.code, 0);
    assert!(out.out().contains("No tools are built in yet."));
}

#[test]
fn info_describes_parameters() {
    let out = cli(&["info", "text.repeat"]);
    assert_eq!(out.code, 0);
    let text = out.out();
    assert!(text.contains("text.repeat [text]"));
    assert!(text.contains("int, 1..5, default 2"));
    assert!(text.contains("text, default \"\""));
    assert!(text.contains("bool, default false"));
    assert!(cli(&["dev.text", "--help"]).out().contains("No parameters."));
    assert_eq!(cli(&["text.repeat", "-h"]).out(), text);
}

#[test]
fn runs_with_defaults() {
    let out = cli(&["text.repeat", "-t", "ab"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.out(), "abab\n");
}

#[test]
fn runs_with_parameters() {
    let out =
        cli(&["text.repeat", "-t", "ab", "-p", "times=3", "-p", "separator=-", "-p", "upper"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.out(), "AB-AB-AB\n");
    let out = cli(&["text.repeat", "--text=x", "--param=times=1", "--param", "upper=no"]);
    assert_eq!(out.out(), "x\n");
}

#[test]
fn reads_stdin() {
    let out = cli_with(&registry(), &["text.repeat", "-i", "-"], b"xy");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.out(), "xyxy\n");
}

#[test]
fn reads_and_writes_files() {
    let tmp = TempDir::new();
    std::fs::write(tmp.path("in.txt"), "ab").unwrap();
    let out_path = tmp.path("out.txt");
    let out = cli(&["text.repeat", "-i", &tmp.path("in.txt"), "-o", &out_path]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.is_empty());
    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "abab");
}

#[test]
fn does_not_overwrite_without_force() {
    let tmp = TempDir::new();
    let out_path = tmp.path("out.txt");
    std::fs::write(&out_path, "old").unwrap();
    let out = cli(&["dev.text", "-o", &out_path]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("already exists"));
    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "old");
    let out = cli(&["dev.text", "-o", &out_path, "--force"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "hello");
}

#[test]
fn single_result_goes_into_an_existing_directory() {
    let tmp = TempDir::new();
    let out = cli(&["dev.text", "-o", &tmp.path("")]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(tmp.path("output-1")).unwrap(), "hello");
}

#[test]
fn several_results_go_into_a_directory_with_safe_names() {
    let tmp = TempDir::new();
    let dir = tmp.path("results");
    let out = cli(&["dev.named", "-o", &dir]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(tmp.path("results/one.txt")).unwrap(), "one");
    assert_eq!(std::fs::read_to_string(tmp.path("results/two.txt")).unwrap(), "two");
    assert!(!tmp.0.join("one.txt").exists(), "a tool-supplied name escaped the directory");
}

#[test]
fn several_results_need_a_directory() {
    let tmp = TempDir::new();
    let file = tmp.path("plain.txt");
    std::fs::write(&file, "x").unwrap();
    let out = cli(&["dev.named", "-o", &file]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("not a directory"));
}

#[test]
fn binary_results_are_not_printed_to_the_terminal() {
    let out = cli(&["dev.binary"]);
    assert_eq!(out.code, 1);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.contains("binary"));
    let out = cli(&["dev.binary", "-o", "-"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.stdout, [0xff, 0xfe, 0x00]);
}

#[test]
fn bad_usage_exits_with_2() {
    let out = cli(&["nope"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("unknown tool `nope`"));
    assert!(out.stderr.contains("sundry list"));
    for args in [
        &["text.repeat", "-t", "x", "-p", "times=abc"][..],
        &["text.repeat", "-t", "x", "-p", "times=99"],
        &["text.repeat", "-t", "x", "-p", "nope=1"],
        &["text.repeat", "-t", "x", "-p", "times=1", "-p", "times=2"],
        &["text.repeat", "-t", "x", "-p", "times"],
        &["text.repeat", "-t", "x", "-p", "upper=maybe"],
        &["text.repeat", "--bogus"],
    ] {
        let out = cli(args);
        assert_eq!(out.code, 2, "args: {args:?}, stderr: {}", out.stderr);
        assert!(out.stdout.is_empty());
    }
}

#[test]
fn runtime_failures_exit_with_1() {
    let out = cli(&["text.repeat"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("no input items"));
    let out = cli(&["text.repeat", "-i", "definitely/missing/file.txt"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("definitely/missing/file.txt"));
}

#[test]
fn built_in_data_converter_works_end_to_end() {
    let registry = sundry_cli::builtin::registry().unwrap();
    let out = cli_with(
        &registry,
        &["convert.data", "-p", "value=15", "-p", "from=kB", "-p", "to=Mbit"],
        b"",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.out(), "15 kB = 0.12 Mbit\n");
    let out = cli_with(&registry, &["convert.data", "-p", "value=1,5", "-p", "to=B"], b"");
    assert_eq!(out.code, 2, "a decimal comma is not accepted on the command line");
    let out = cli_with(&registry, &["convert.data", "-p", "from=parsec"], b"");
    assert_eq!(out.code, 2);
    assert!(cli_with(&registry, &["list", "-c", "convert"], b"").out().contains("convert.data"));
}
