//! Image tools. `register` adds all of them to a registry.

pub mod codec;
pub mod plan;
pub mod preview;
pub mod resize;

use sundry_core::{Registry, Result};

pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(resize::Resize)
}
