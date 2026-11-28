//! Image tools. `register` adds all of them to a registry.

pub mod codec;
pub mod convert;
pub mod crop;
pub mod flip;
pub mod metadata;
pub mod plan;
pub mod preview;
pub mod resize;
pub mod rotate;
pub mod round;

use sundry_core::{Error, Input, Item, Job, Output, Registry, Result};

pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(resize::Resize)?;
    registry.register(convert::Convert)?;
    registry.register(round::RoundCorners)?;
    registry.register(crop::Crop)?;
    registry.register(rotate::Rotate)?;
    registry.register(flip::Flip)?;
    registry.register(metadata::Metadata)
}

/// Runs `f` on every item in order; errors get the file name as a prefix.
pub(crate) fn map_items(
    input: &Input,
    job: &Job,
    f: impl Fn(&Item) -> Result<Item>,
) -> Result<Output> {
    let total = input.items.len();
    let mut items = Vec::with_capacity(total);
    for (i, item) in input.items.iter().enumerate() {
        job.check_cancelled()?;
        let done = f(item).map_err(|err| match (&item.name, err) {
            (_, Error::Cancelled) => Error::Cancelled,
            (Some(name), err) => Error::Processing(format!("{name}: {err}")),
            (None, err) => err,
        })?;
        items.push(done);
        // A batch runner passes single items and reports the batch progress itself.
        if total > 1 {
            job.set_progress((i + 1) as f32 / total as f32);
        }
    }
    Ok(Output { items })
}
