fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::compile_protos("response_metadata.proto")?;
  Ok(())
}
