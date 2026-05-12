use std::path::{Path, PathBuf};

use anyhow::anyhow;
use md5::Digest;
use prost::Message;
use prost_types::FileDescriptorSet;
use tracing::trace;

/// Parse a proto file into a FileDescriptorSet using the embedded protox compiler.
///
/// Returns the descriptor set, its md5 digest, and the raw encoded bytes.
pub(crate) fn parse_proto_file(
  proto_file: &Path,
  additional_includes: &[String],
) -> anyhow::Result<(FileDescriptorSet, Digest, Vec<u8>)> {
  trace!(proto_file = ?proto_file, additional_includes = ?additional_includes, "Parsing proto file");

  let parent_dir = proto_file.parent().unwrap_or(Path::new("."));
  let file_name = proto_file.file_name()
    .and_then(|n| n.to_str())
    .ok_or_else(|| anyhow!("Proto file has no valid file name: {:?}", proto_file))?;

  let mut include_dirs: Vec<PathBuf> = vec![parent_dir.to_path_buf()];
  for inc in additional_includes {
    include_dirs.push(PathBuf::from(inc));
  }

  let mut compiler = protox::Compiler::new(include_dirs)?;
  compiler.open_file(file_name)?;

  let descriptor_set = compiler.file_descriptor_set();
  let descriptor_bytes = descriptor_set.encode_to_vec();
  let digest = md5::compute(&descriptor_bytes);

  Ok((descriptor_set, digest, descriptor_bytes))
}
