fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = fusor_build::backend::build::BuildInputs::from_cargo(["ui/panel.html"])?;
    fusor_build::backend::build::compile_cargo(&inputs, "memory", |_, html| {
        fusor_build::backend::generate(html, &memory_compiler::Memory)
    })?;
    if std::env::var_os("CARGO_FEATURE_BROWSER").is_some() {
        fusor_build::compile_app()?;
    }
    Ok(())
}
