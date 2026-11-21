//! Converter tools. `register` adds all of them to a registry.

pub mod data_size;

use sundry_core::{Registry, Result};

pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(data_size::DataSize)
}
