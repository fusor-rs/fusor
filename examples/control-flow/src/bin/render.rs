#[cfg(not(target_arch = "wasm32"))]
fn main() -> fusor_server::Result<()> {
    use fusor_server::{Context, Render};
    let html = fusor_control_flow::Shared::new().render(&mut Context::new())?;
    println!("{html}");
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
