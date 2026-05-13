use tonic_prost_build::compile_protos;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  compile_protos("repeated_enum.proto")?;
  Ok(())
}
