fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fixed style: only `Palette` (system theme detection) is taken from it.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app-window.slint", config)?;
    Ok(())
}
