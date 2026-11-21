fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fixed style: only `Palette` (system theme detection) is taken from it.
    // Translations are bundled from lang/<locale>/LC_MESSAGES/<crate>.po without contexts.
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app-window.slint", config)?;
    Ok(())
}
