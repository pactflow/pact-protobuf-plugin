fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::configure().include_file("mod.rs").compile(
    &["primary/primary.proto", "imported/imported.proto"],
    &["."],
  )?;
  Ok(())
}
