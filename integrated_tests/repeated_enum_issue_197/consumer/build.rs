fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::compile_protos("../repeated_enum.proto")?;
  Ok(())
}
