fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().include_file("mod.rs").compile(
        &[
            "primary/primary.proto",
            "primary/rectangle.proto",
            "imported/imported.proto",
            "zimported/zimported.proto",
        ],
        &["."],
    )?;
    Ok(())
}
