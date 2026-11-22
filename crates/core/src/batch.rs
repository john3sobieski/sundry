//! Batch runs: one tool and one parameter set applied to many independent inputs.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::data::{Input, Item, Output};
use crate::error::{Error, Result};
use crate::job::Job;
use crate::params::Params;
use crate::tool::Tool;

/// Where one input comes from. The bytes are loaded only when a worker needs
/// them, so a batch of hundreds of files never holds all of them in memory.
pub struct BatchSource {
    pub name: String,
    load: Box<dyn Fn() -> Result<Vec<u8>> + Send + Sync>,
}

impl BatchSource {
    pub fn new(
        name: impl Into<String>,
        load: impl Fn() -> Result<Vec<u8>> + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), load: Box::new(load) }
    }

    /// A source that already holds its bytes (tests, in-memory data).
    pub fn from_bytes(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self::new(name, move || Ok(bytes.clone()))
    }
}

/// How a batch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchReport {
    pub ok: usize,
    pub failed: usize,
    /// Set when the job was cancelled before every input was processed.
    pub cancelled: bool,
}

/// Runs `tool` on every source with `workers` threads. `params` must already be
/// resolved (see `params::resolve`). Each input is a separate run, so a broken
/// file fails only itself. `on_item(index, result)` is called from a worker
/// thread as soon as an input is done, in no particular order; the batch's
/// progress goes to `job`.
pub fn run_batch(
    tool: &dyn Tool,
    params: &Params,
    sources: &[BatchSource],
    job: &Job,
    workers: usize,
    on_item: &(dyn Fn(usize, Result<Output>) + Sync),
) -> BatchReport {
    let total = sources.len();
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let ok = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, total.max(1)) {
            scope.spawn(|| {
                while !job.is_cancelled() {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(source) = sources.get(index) else { break };
                    let result = process(tool, params, source, job);
                    match &result {
                        Ok(_) => ok.fetch_add(1, Ordering::Relaxed),
                        Err(Error::Cancelled) => skipped.fetch_add(1, Ordering::Relaxed),
                        Err(_) => failed.fetch_add(1, Ordering::Relaxed),
                    };
                    if matches!(result, Err(Error::Cancelled)) {
                        break;
                    }
                    on_item(index, result);
                    let done = finished.fetch_add(1, Ordering::Relaxed) + 1;
                    job.set_progress(done as f32 / total as f32);
                }
            });
        }
    });

    BatchReport {
        ok: ok.into_inner(),
        failed: failed.into_inner(),
        cancelled: job.is_cancelled() || skipped.into_inner() > 0,
    }
}

/// Loads and processes one input; a panic inside a codec becomes an error
/// for this input instead of taking the whole batch down.
fn process(tool: &dyn Tool, params: &Params, source: &BatchSource, job: &Job) -> Result<Output> {
    let run = || {
        let bytes = (source.load)()?;
        let item = Item::bytes(bytes).named(source.name.clone());
        tool.run(&Input::single(item), params, job)
    };
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| {
        Err(Error::Processing(format!("internal error while processing {}", source.name)))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::tool::{Category, ToolInfo};

    /// Reverses the bytes; fails on empty input and panics on `b"boom"`.
    struct Reverse;

    impl Tool for Reverse {
        fn info(&self) -> ToolInfo {
            ToolInfo::new("test.reverse", Category::Dev, "Reverse", "").per_file()
        }

        fn run(&self, input: &Input, _: &Params, _: &Job) -> Result<Output> {
            let item = input.first()?;
            let bytes = item.data.as_bytes();
            if bytes.is_empty() {
                return Err(Error::InvalidInput("empty".to_owned()));
            }
            assert!(bytes != b"boom", "boom");
            let reversed: Vec<u8> = bytes.iter().rev().copied().collect();
            Ok(Output::single(Item::bytes(reversed).named(item.name.clone().unwrap_or_default())))
        }
    }

    fn collect(
        sources: &[BatchSource],
        job: &Job,
        workers: usize,
    ) -> (BatchReport, Vec<(usize, Result<Output>)>) {
        let results = Mutex::new(Vec::new());
        let report = run_batch(&Reverse, &Params::new(), sources, job, workers, &|i, r| {
            results.lock().unwrap().push((i, r));
        });
        let mut results = results.into_inner().unwrap();
        results.sort_by_key(|(i, _)| *i);
        (report, results)
    }

    #[test]
    fn every_input_is_processed_once() {
        let sources: Vec<_> =
            (0..50u8).map(|i| BatchSource::from_bytes(format!("f{i}"), vec![i, 1, 2])).collect();
        let job = Job::new();
        let (report, results) = collect(&sources, &job, 4);
        assert_eq!(report, BatchReport { ok: 50, failed: 0, cancelled: false });
        assert_eq!(results.len(), 50);
        for (i, result) in results {
            let out = result.unwrap();
            assert_eq!(out.items[0].data.as_bytes(), &[2, 1, i as u8]);
            assert_eq!(out.items[0].name.as_deref(), Some(format!("f{i}").as_str()));
        }
        assert_eq!(job.progress(), 1.0);
    }

    #[test]
    fn one_bad_input_does_not_stop_the_others() {
        let sources = vec![
            BatchSource::from_bytes("a", vec![1, 2]),
            BatchSource::from_bytes("empty", vec![]),
            BatchSource::from_bytes("boom", b"boom".to_vec()),
            BatchSource::new("unreadable", || Err(Error::InvalidInput("gone".to_owned()))),
            BatchSource::from_bytes("b", vec![3]),
        ];
        let (report, results) = collect(&sources, &Job::new(), 2);
        assert_eq!(report, BatchReport { ok: 2, failed: 3, cancelled: false });
        assert!(results[0].1.is_ok() && results[4].1.is_ok());
        assert!(matches!(results[2].1, Err(Error::Processing(_))));
    }

    #[test]
    fn a_cancelled_job_runs_nothing() {
        let sources = vec![BatchSource::from_bytes("a", vec![1])];
        let job = Job::new();
        job.cancel();
        let (report, results) = collect(&sources, &job, 3);
        assert_eq!(report, BatchReport { ok: 0, failed: 0, cancelled: true });
        assert!(results.is_empty());
    }

    #[test]
    fn empty_batch_is_fine() {
        let (report, results) = collect(&[], &Job::new(), 8);
        assert_eq!(report, BatchReport::default());
        assert!(results.is_empty());
    }
}
