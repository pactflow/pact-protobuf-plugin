use std::path::{Path, PathBuf};

use anyhow::anyhow;
use md5::Digest;
use prost::Message;
use prost_types::FileDescriptorSet;
use tracing::trace;

/// Parse a proto file into a FileDescriptorSet using the embedded protox compiler.
///
/// Returns the descriptor set, its md5 digest, and the raw encoded bytes.
/// The descriptor set includes all transitively imported files, equivalent to
/// protoc's --include_imports flag.
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

  // protox::compile includes all transitively imported files in the returned set,
  // matching the behaviour of `protoc --include_imports`.
  let descriptor_set = protox::compile([file_name], include_dirs)?;
  let descriptor_bytes = descriptor_set.encode_to_vec();
  let digest = md5::compute(&descriptor_bytes);

  Ok((descriptor_set, digest, descriptor_bytes))
}

#[cfg(test)]
mod tests {
  use super::*;
  use expectest::prelude::*;
  use std::io::Write;
  use tempfile::TempDir;

  #[test]
  fn parse_proto_file_with_no_imports() {
    let proto_file = Path::new("tests/basic_values.proto").canonicalize().unwrap();
    let (fds, _, bytes) = parse_proto_file(&proto_file, &[]).unwrap();

    // Single file, no imports
    expect!(fds.file.len()).to(be_equal_to(1));
    expect!(fds.file[0].name.as_deref()).to(be_some().value("basic_values.proto"));
    expect!(bytes.is_empty()).to(be_false());
  }

  #[test]
  fn parse_proto_file_includes_transitive_imports() {
    let proto_file = Path::new("tests/enum.proto").canonicalize().unwrap();
    let (fds, _, _) = parse_proto_file(&proto_file, &[]).unwrap();

    let file_names: Vec<_> = fds.file.iter()
      .filter_map(|f| f.name.as_deref())
      .collect();

    // enum.proto imports enum_imported.proto — both must appear in the set
    expect!(file_names.iter().any(|n| n.contains("enum_imported"))).to(be_true());

    // Values2, defined in enum_imported.proto, must be resolvable
    let has_values2 = fds.file.iter()
      .any(|f| f.enum_type.iter().any(|e| e.name.as_deref() == Some("Values2")));
    expect!(has_values2).to(be_true());
  }

  #[test]
  fn parse_proto_file_resolves_imports_via_additional_includes() {
    // Create a temp dir with a proto file that imports from the tests/ dir.
    // The import cannot be resolved without additional_includes pointing at tests/.
    let tmp = TempDir::new().unwrap();
    let mut f = std::fs::File::create(tmp.path().join("main.proto")).unwrap();
    writeln!(f, r#"syntax = "proto3"; package test; import "enum_imported.proto"; message Wrap {{ .example.enum.package.Values2 v = 1; }}"#).unwrap();
    drop(f);

    let tests_dir = Path::new("tests").canonicalize().unwrap().to_string_lossy().to_string();
    let proto_file = tmp.path().join("main.proto");
    let (fds, _, _) = parse_proto_file(&proto_file, &[tests_dir]).unwrap();

    let has_values2 = fds.file.iter()
      .any(|f| f.enum_type.iter().any(|e| e.name.as_deref() == Some("Values2")));
    expect!(has_values2).to(be_true());
  }

  #[test]
  fn parse_proto_file_digest_is_stable() {
    let proto_file = Path::new("tests/basic_values.proto").canonicalize().unwrap();
    let (_, digest1, bytes1) = parse_proto_file(&proto_file, &[]).unwrap();
    let (_, digest2, bytes2) = parse_proto_file(&proto_file, &[]).unwrap();

    expect!(format!("{:x}", digest1)).to(be_equal_to(format!("{:x}", digest2)));
    expect!(bytes1).to(be_equal_to(bytes2));
  }

  #[test]
  fn parse_proto_file_errors_on_nonexistent_file() {
    let result = parse_proto_file(Path::new("/nonexistent/path/to/missing.proto"), &[]);
    expect!(result.is_err()).to(be_true());
  }
}
