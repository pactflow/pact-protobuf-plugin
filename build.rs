use os_info::Type::Alpine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let os_info = os_info::get();
  if os_info.os_type() != Alpine {
    // This causes a seg violation on Alpine
    built::write_built_file()?;
  }
  Ok(())
}
