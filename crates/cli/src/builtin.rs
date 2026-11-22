//! The tools shipped with the app. Each tool crate adds its registration here.

use sundry_core::{Registry, Result};

pub fn registry() -> Result<Registry> {
    let mut registry = Registry::new();
    sundry_tools_convert::register(&mut registry)?;
    sundry_tools_image::register(&mut registry)?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use crate::args::RESERVED_IDS;

    #[test]
    fn tool_ids_do_not_clash_with_cli_commands() {
        let registry = super::registry().unwrap();
        for entry in registry.iter() {
            let id = entry.info().id.as_str();
            assert!(!RESERVED_IDS.contains(&id), "`{id}` is a CLI command");
        }
    }
}
