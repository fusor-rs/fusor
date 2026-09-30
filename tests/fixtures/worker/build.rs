fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rustc-check-cfg=cfg(worker_config)");
    fusor_build::compile_app()
}
