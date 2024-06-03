fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().include_file("mod.rs").compile(
        &[
            "primary/service.proto",
            "primary/rectangle.proto",
            "primary/request.proto",
            "primary/response.proto",
            "imported/imported.proto",
            "zimported/zimported.proto",
        ],
        &["."],
    )?;
    Ok(())
}
