fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::compile_protos("../list_appender.proto")?;
  Ok(())
}
