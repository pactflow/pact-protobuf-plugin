fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().include_file("mod.rs").compile(
        &[
            "primary/service.proto",
        ],
        &[".", "./primary", "./imported", "./no_package"],
    )?;
    Ok(())
}
