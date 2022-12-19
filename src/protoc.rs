use std::collections::HashMap;
use std::env::consts::{ARCH, OS};
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::ops::Add;
use std::path::{Path, PathBuf};
use std::str::from_utf8;

use anyhow::anyhow;
use futures::TryFutureExt;
use md5::Digest;
use os_info::{Bitness, Info, Type};
use pact_models::json_utils::json_to_string;
use prost::Message;
use prost_types::FileDescriptorSet;
use reqwest::Url;
use serde_json::Value;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tracing::{debug, error, trace};
use zip::ZipArchive;

/// Encapsulation over the Protocol Buffers compiler.
pub(crate) struct Protoc {
  protoc_path: String,
  local_install: bool,
  additional_includes: Vec<String>
}

impl Protoc {
  /// Create a new Protoc
  fn new(path: String, local_install: bool, additional_includes: Vec<String>) -> Self {
    Protoc {
      protoc_path: path,
      local_install,
      additional_includes
    }
  }

  /// Try to invoke the protoc binary
  async fn invoke(&self) -> anyhow::Result<String> {
    debug!("Invoking protoc: '{} --version'", self.protoc_path);
    match Command::new(&self.protoc_path).arg("--version").output().await {
      Ok(out) => {
        if out.status.success() {
          let version = from_utf8(out.stdout.as_ref()).unwrap_or_default();
          debug!("Protoc binary invoked OK: {}", version);
          Ok(version.to_string())
        } else {
          error!("Protoc output: {}", from_utf8(out.stdout.as_slice()).unwrap_or_default());
          error!("Protoc stderr: {}", from_utf8(out.stderr.as_slice()).unwrap_or_default());
          Err(anyhow!("Failed to invoke protoc binary: exit code {}", out.status))
        }
      }
      Err(err) => Err(anyhow!("Failed to invoke protoc binary: {}", err))
    }
  }

  /// Get protoc to compile the proto file, and the load the file descriptors
  pub(crate) async fn parse_proto_file(&self, proto_file: &Path) -> anyhow::Result<(FileDescriptorSet, Digest, Vec<u8>)> {
    trace!(proto_file = ?proto_file, additional_includes = ?self.additional_includes, "Parsing proto file");
    let tmp_dir = Path::new("tmp");
    fs::create_dir_all(tmp_dir)?;
    let file = NamedTempFile::new_in(tmp_dir)?;
    let output = format!("-o{}", file.path().to_string_lossy());

    // Protoc does not work with Windows \\?\ paths
    let path_str = proto_file.to_string_lossy();
    let path_str = path_str.strip_prefix(r"\\?\").unwrap_or(&*path_str);
    let proto_file = PathBuf::from(path_str);

    let mut parent_dir = proto_file.to_path_buf();
    parent_dir.pop();
    let include = format!("-I{}", parent_dir.to_string_lossy());

    // Create the protoc command line to invoke
    let mut cmd = Command::new(&self.protoc_path);
    cmd.arg(output.as_str())
      .arg(include.as_str())
      .arg("--include_imports")
      .arg(proto_file.clone());

    // Add any additional includes defined by the user
    for inc in &self.additional_includes {
      cmd.arg(format!("-I{}", inc));
    }

    // If it is a local install, the default Protobuf well-defined types will be available
    if self.local_install {
      let include_path = PathBuf::from("protoc").join("include");
      let include2 = format!("-I{}", include_path.to_string_lossy());
      cmd.arg(include2);
    }

    debug!("Invoking protoc: {:?}", cmd);
    match cmd.output().await {
      Ok(out) => {
        if out.status.success() {
          let data = fs::read(file.path())?;
          FileDescriptorSet::decode(data.as_slice())
            .map(|descriptor| (descriptor, md5::compute(data.as_slice()), data))
            .map_err(|err| anyhow!("Failed to load file descriptor set - {}", err))
        } else {
          error!("Protoc output: {}", from_utf8(out.stdout.as_slice()).unwrap_or_default());
          error!("Protoc stderr: {}", from_utf8(out.stderr.as_slice()).unwrap_or_default());
          Err(anyhow!("Failed to invoke protoc binary: exit code {}", out.status))
        }
      }
      Err(err) => Err(anyhow!("Failed to invoke protoc binary: {}", err))
    }
  }
}

// This function first checks for an unpacked protoc binary, and tries to run that
// otherwise it will try unpack the version for the current OS
// otherwise it will try download and unpack the version for the current OS
// otherwise then fallback to any version on the system path
// will error if unable to do that
pub(crate) async fn setup_protoc(config: &HashMap<String, Value>, additional_includes: &Vec<String>) -> anyhow::Result<Protoc> {
  trace!(config = ?config, additional_includes = ?additional_includes, "Setting up protoc");
  let os_info = os_info::get();
  debug!("Detected OS: {}", os_info);

  local_protoc(&os_info, additional_includes)
    .or_else(|err| {
      trace!("local_protoc: {}", err);
      unpack_protoc(config, &os_info, additional_includes)
    })
    .or_else(|err| {
      trace!("unpack_protoc: {}", err);
      download_protoc(config, &os_info, additional_includes)
    })
    .or_else(|err| {
      trace!("download_protoc: {}", err);
      system_protoc(additional_includes)
    })
    .await
}

async fn download_protoc(
  config: &HashMap<String, Value>,
  os_info: &Info,
  additional_includes: &Vec<String>
) -> anyhow::Result<Protoc> {
  trace!("download_protoc: config = {:?}", config);
  let protoc_version = config.get("protocVersion")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Could not get the protoc version from the manifest"))?;
  let download_url = config.get("downloadUrl")
    .map(|v| {
      let url = json_to_string(v);
      if url.ends_with('/') {
        url
      } else {
        url.add("/")
      }
    })
    .ok_or_else(|| anyhow!("Could not get the protoc download URL from the manifest"))?;
  let base_url = Url::parse(download_url.as_str())?;
  let os_type = os_type(os_info.bitness(), ARCH, OS);
  let url = base_url.join(format!("v{}/protoc-{}-{}.zip", protoc_version, protoc_version, os_type).as_str())?;

  debug!("Downloading protoc from '{}'", url);
  let mut response = reqwest::get(url).await?;

  if response.status().is_success() {
    let mut protoc_file = File::create(format!("./protoc-{}-{}.zip", protoc_version, os_type))?;
    let mut count: usize = 0;
    while let Some(chunk) = response.chunk().await? {
      count += chunk.len();
      protoc_file.write_all(chunk.as_ref())?;
    }
    debug!("Downloaded {} bytes", count);
    unpack_protoc(config, os_info, additional_includes).await
  } else {
    Err(anyhow!("Failed to download protoc - {}", response.status()))
  }
}

async fn system_protoc(additional_includes: &[String]) -> anyhow::Result<Protoc> {
  trace!("system_protoc: looking for protoc in system path");
  let program = if OS == "windows" { "where" } else { "which" };
  match Command::new(program).arg("protoc").output().await {
    Ok(out) => {
      if out.status.success() {
        let path = from_utf8(out.stdout.as_ref())?;
        debug!("Found protoc binary: {}", path);
        let protoc = Protoc::new(path.trim().to_string(), false, additional_includes.to_owned());
        protoc.invoke().await?;
        Ok(protoc)
      } else {
        debug!("{} output: {}", program, from_utf8(out.stdout.as_slice()).unwrap_or_default());
        debug!("{} stderr: {}", program, from_utf8(out.stderr.as_slice()).unwrap_or_default());
        Err(anyhow!("Failed to invoke {}: exit code {}", program, out.status))
      }
    }
    Err(err) => Err(anyhow!("Failed to find system protoc binary: {}", err))
  }
}

async fn local_protoc(os_info: &Info, additional_includes: &[String]) -> anyhow::Result<Protoc> {
  let path = PathBuf::from(".")
      .join("protoc")
      .join("bin");
  let local_path = if os_info.os_type() == Type::Windows {
    path.join("protoc.exe")
  } else {
    path.join("protoc")
  };
  let path_str = local_path.to_string_lossy();
  trace!("Looking for local protoc at '{}'", path_str);
  if local_path.exists() {
    debug!("Found unpacked protoc binary");
    let protoc = Protoc::new(path_str.to_string(), true, additional_includes.to_owned());
    protoc.invoke().await?;
    Ok(protoc)
  } else {
    trace!("No local unpacked protoc binary");
    Err(anyhow!("No local unpacked protoc binary"))
  }
}

async fn unpack_protoc(
  config: &HashMap<String, Value>,
  os_info: &Info,
  additional_includes: &[String]
) -> anyhow::Result<Protoc> {
  let protoc_version = config.get("protocVersion")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Could not get the protoc version from the manifest"))?;
  let protoc_file = format!("./protoc-{}-{}.zip", protoc_version, os_type(os_info.bitness(), ARCH, OS));
  trace!("Looking for protoc zip archive '{}'", protoc_file);
  let protoc_zip_path = Path::new(protoc_file.as_str());
  if protoc_zip_path.exists() {
    debug!("Found protoc zip archive: {}", protoc_zip_path.to_string_lossy());
    unzip_proto_archive(protoc_zip_path)?;
    local_protoc(os_info, additional_includes).await
  } else {
    trace!("protoc zip archive not found");
    Err(anyhow!("No local protoc zip archive"))
  }
}

fn unzip_proto_archive(archive_path: &Path) -> anyhow::Result<()> {
  let file = File::open(archive_path)?;
  let mut archive = ZipArchive::new(file)?;
  let base_path = PathBuf::from("protoc");
  for i in 0..archive.len() {
    let mut file = archive.by_index(i)?;
    let outpath = match file.enclosed_name() {
      Some(path) => base_path.join(path).to_owned(),
      None => {
        trace!("Skipping file {} as it is not a valid file name", i);
        continue
      }
    };

    if file.name().ends_with('/') {
      trace!("Directory {} extracted to \"{}\"", i, outpath.display());
      fs::create_dir_all(&outpath)?;
    } else {
      trace!("File {} extracted to \"{}\" ({} bytes)", i, outpath.display(), file.size());
      if let Some(p) = outpath.parent() {
        if !p.exists() {
          fs::create_dir_all(p)?;
        }
      }
      let mut outfile = fs::File::create(&outpath)?;
      io::copy(&mut file, &mut outfile)?;
    }

    // Get and Set permissions
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      if let Some(mode) = file.unix_mode() {
        fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))?;
      }
    }
  }
  Ok(())
}

fn os_type(os_info: Bitness, arch: &str, os: &str) -> String {
  match os {
    "linux" => match arch {
      "x86" => "linux-x86_32",
      "x86_64" => "linux-x86_64",
      "aarch64" => "linux-aarch_64",
      "s390x" => "linux-s390_64",
      _ => "unknown"
    }.to_string(),
    "macos" => format!("osx-{}", arch),
    "windows" => format!("win{}", match os_info {
      Bitness::X32 => "32",
      Bitness::X64 => "64",
      _ => "64"
    }),
    _ => "unknown".to_string()
  }
}

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use os_info::Bitness;

  use super::os_type;

  #[test]
  fn os_type_test() {
    expect!(os_type(Bitness::X32, "x86", "linux").as_str()).to(be_equal_to("linux-x86_32"));
    expect!(os_type(Bitness::X64, "x86_64", "linux").as_str()).to(be_equal_to("linux-x86_64"));
    expect!(os_type(Bitness::X64, "aarch64", "linux").as_str()).to(be_equal_to("linux-aarch_64"));
    expect!(os_type(Bitness::X64, "x86_64", "macos").as_str()).to(be_equal_to("osx-x86_64"));
    expect!(os_type(Bitness::X32, "", "windows").as_str()).to(be_equal_to("win32"));
    expect!(os_type(Bitness::X64, "", "windows").as_str()).to(be_equal_to("win64"));
  }
}
