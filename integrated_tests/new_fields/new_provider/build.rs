fn main() -> Result<(), Box<dyn std::error::Error>> {
  tonic_build::compile_protos("../new_fields_2.proto")?;
  Ok(())
}
