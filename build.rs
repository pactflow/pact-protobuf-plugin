use std::{env, fs, path};

use os_info::Type::Alpine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let os_info = os_info::get();
  if os_info.os_type() != Alpine {
    // This causes a seg violation on Alpine
    built::write_built_file()?;
  } else {
    let dst = path::Path::new(&env::var("OUT_DIR").unwrap()).join("built.rs");
    fs::File::create(&dst)?;
  }
  Ok(())
}
