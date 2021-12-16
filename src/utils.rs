//! Shared utilities

use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::anyhow;
use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorSet, MessageOptions, Type, Value};
use prost_types::field_descriptor_proto::Label;

use crate::message_decoder::ProtobufField;

/// Return the last name in a dot separated string
pub fn last_name(entry_type_name: &str) -> &str {
  entry_type_name.split('.').last().unwrap_or_else(|| entry_type_name)
}

/// Convert a Protobuf Struct to a BTree Map
pub fn proto_struct_to_btreemap(val: &prost_types::Struct) -> BTreeMap<String, Value> {
  val.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Search for a message by type name in the descriptors
pub fn find_message_type_by_name(message_name: &str, descriptors: &FileDescriptorSet) -> anyhow::Result<DescriptorProto> {
  descriptors.file.iter()
    .map(|descriptor| descriptor.message_type.iter().find(|message| message.name.clone().unwrap_or_default() == message_name))
    .find(|result| result.is_some())
    .flatten()
    .cloned()
    .ok_or_else(|| anyhow!("Did not find a message type '{}' in the descriptors", message_name))
}

/// If the field is a map field. A field will be a map field if it is a repeated field, the field
/// type is a message and the nested type has the map flag set on the message options.
pub fn is_map_field(message_descriptor: &DescriptorProto, field: &FieldDescriptorProto) -> bool {
  if field.label() == Label::Repeated && field.r#type() == field_descriptor_proto::Type::Message {
    match find_nested_type(message_descriptor, field) {
      Some(nested) => match nested.options {
        None => false,
        Some(options) => options.map_entry.unwrap_or(false)
      },
      None => false
    }
  } else {
    false
  }
}

/// Returns the nested descriptor for this field. Field must be an embedded message.
pub fn find_nested_type(message_descriptor: &DescriptorProto, field: &FieldDescriptorProto) -> Option<DescriptorProto> {
  if field.r#type() == field_descriptor_proto::Type::Message {
    let type_name = field.type_name.clone().unwrap_or_default();
    let message_type = last_name(type_name.as_str());
    message_descriptor.nested_type.iter().find(|nested| {
      nested.name.clone().unwrap_or_default() == message_type
    }).cloned()
  } else {
    None
  }
}

/// Return the hexadecimal representation for the bytes
pub(crate) fn as_hex(data: &[u8]) -> String {
  let mut buffer = String::with_capacity(data.len() * 2);

  for b in data {
    let _ = write!(&mut buffer, "{:02x}", b);
  }

  buffer
}

/// If the message fields include the field with the given descriptor
pub fn find_message_field<'a>(message: &'a Vec<ProtobufField>, field_descriptor: &ProtobufField) -> Option<&'a ProtobufField> {
  message.iter().find(|v| v.field_num == field_descriptor.field_num)
}

/// If the field is a repeated field
pub fn is_repeated(descriptor: &FieldDescriptorProto) -> bool {
  descriptor.label() == Label::Repeated
}

#[cfg(test)]
mod tests {
  use bytes::Bytes;
  use expectest::prelude::*;
  use prost::Message;
  use prost_types::{DescriptorProto, field, FieldDescriptorProto, FileDescriptorSet, MessageOptions};
  use prost_types::field_descriptor_proto::{Label, Type};

  use crate::utils::{as_hex, find_message_type_by_name, find_nested_type, is_map_field, last_name};

  #[test]
  fn last_name_test() {
    expect!(last_name("")).to(be_equal_to(""));
    expect!(last_name("test")).to(be_equal_to("test"));
    expect!(last_name(".")).to(be_equal_to(""));
    expect!(last_name("test.")).to(be_equal_to(""));
    expect!(last_name(".test")).to(be_equal_to("test"));
    expect!(last_name("1.2")).to(be_equal_to("2"));
    expect!(last_name("1.2.3.4")).to(be_equal_to("4"));
  }

  const DESCRIPTORS: &'static str = "CuIFChxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvEg9nb29\
    nbGUucHJvdG9idWYimAEKBlN0cnVjdBI7CgZmaWVsZHMYASADKAsyIy5nb29nbGUucHJvdG9idWYuU3RydWN0LkZpZWxkc\
    0VudHJ5UgZmaWVsZHMaUQoLRmllbGRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLAoFdmFsdWUYAiABKAsyFi5nb29nbGU\
    ucHJvdG9idWYuVmFsdWVSBXZhbHVlOgI4ASKyAgoFVmFsdWUSOwoKbnVsbF92YWx1ZRgBIAEoDjIaLmdvb2dsZS5wcm90b2\
    J1Zi5OdWxsVmFsdWVIAFIJbnVsbFZhbHVlEiMKDG51bWJlcl92YWx1ZRgCIAEoAUgAUgtudW1iZXJWYWx1ZRIjCgxzdHJpb\
    mdfdmFsdWUYAyABKAlIAFILc3RyaW5nVmFsdWUSHwoKYm9vbF92YWx1ZRgEIAEoCEgAUglib29sVmFsdWUSPAoMc3RydWN0\
    X3ZhbHVlGAUgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdEgAUgtzdHJ1Y3RWYWx1ZRI7CgpsaXN0X3ZhbHVlGAYgASg\
    LMhouZ29vZ2xlLnByb3RvYnVmLkxpc3RWYWx1ZUgAUglsaXN0VmFsdWVCBgoEa2luZCI7CglMaXN0VmFsdWUSLgoGdmFsdW\
    VzGAEgAygLMhYuZ29vZ2xlLnByb3RvYnVmLlZhbHVlUgZ2YWx1ZXMqGwoJTnVsbFZhbHVlEg4KCk5VTExfVkFMVUUQAEJ/C\
    hNjb20uZ29vZ2xlLnByb3RvYnVmQgtTdHJ1Y3RQcm90b1ABWi9nb29nbGUuZ29sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9r\
    bm93bi9zdHJ1Y3RwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCoYECh5nb29\
    nbGUvcHJvdG9idWYvd3JhcHBlcnMucHJvdG8SD2dvb2dsZS5wcm90b2J1ZiIjCgtEb3VibGVWYWx1ZRIUCgV2YWx1ZRgBIA\
    EoAVIFdmFsdWUiIgoKRmxvYXRWYWx1ZRIUCgV2YWx1ZRgBIAEoAlIFdmFsdWUiIgoKSW50NjRWYWx1ZRIUCgV2YWx1ZRgBI\
    AEoA1IFdmFsdWUiIwoLVUludDY0VmFsdWUSFAoFdmFsdWUYASABKARSBXZhbHVlIiIKCkludDMyVmFsdWUSFAoFdmFsdWUY\
    ASABKAVSBXZhbHVlIiMKC1VJbnQzMlZhbHVlEhQKBXZhbHVlGAEgASgNUgV2YWx1ZSIhCglCb29sVmFsdWUSFAoFdmFsdWU\
    YASABKAhSBXZhbHVlIiMKC1N0cmluZ1ZhbHVlEhQKBXZhbHVlGAEgASgJUgV2YWx1ZSIiCgpCeXRlc1ZhbHVlEhQKBXZhbH\
    VlGAEgASgMUgV2YWx1ZUKDAQoTY29tLmdvb2dsZS5wcm90b2J1ZkINV3JhcHBlcnNQcm90b1ABWjFnb29nbGUuZ29sYW5nL\
    m9yZy9wcm90b2J1Zi90eXBlcy9rbm93bi93cmFwcGVyc3Bi+AEBogIDR1BCqgIeR29vZ2xlLlByb3RvYnVmLldlbGxLbm93\
    blR5cGVzYgZwcm90bzMKvgEKG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90bxIPZ29vZ2xlLnByb3RvYnVmIgcKBUVtcHR\
    5Qn0KE2NvbS5nb29nbGUucHJvdG9idWZCCkVtcHR5UHJvdG9QAVouZ29vZ2xlLmdvbGFuZy5vcmcvcHJvdG9idWYvdHlwZX\
    Mva25vd24vZW1wdHlwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCv0iCgxwb\
    HVnaW4ucHJvdG8SDmlvLnBhY3QucGx1Z2luGhxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvGh5nb29nbGUvcHJvdG9i\
    dWYvd3JhcHBlcnMucHJvdG8aG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90byJVChFJbml0UGx1Z2luUmVxdWVzdBImCg5\
    pbXBsZW1lbnRhdGlvbhgBIAEoCVIOaW1wbGVtZW50YXRpb24SGAoHdmVyc2lvbhgCIAEoCVIHdmVyc2lvbiLHAgoOQ2F0YW\
    xvZ3VlRW50cnkSPAoEdHlwZRgBIAEoDjIoLmlvLnBhY3QucGx1Z2luLkNhdGFsb2d1ZUVudHJ5LkVudHJ5VHlwZVIEdHlwZ\
    RIQCgNrZXkYAiABKAlSA2tleRJCCgZ2YWx1ZXMYAyADKAsyKi5pby5wYWN0LnBsdWdpbi5DYXRhbG9ndWVFbnRyeS5WYWx1\
    ZXNFbnRyeVIGdmFsdWVzGjkKC1ZhbHVlc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5EhQKBXZhbHVlGAIgASgJUgV2YWx1ZTo\
    COAEiZgoJRW50cnlUeXBlEhMKD0NPTlRFTlRfTUFUQ0hFUhAAEhUKEUNPTlRFTlRfR0VORVJBVE9SEAESDwoLTU9DS19TRV\
    JWRVIQAhILCgdNQVRDSEVSEAMSDwoLSU5URVJBQ1RJT04QBCJSChJJbml0UGx1Z2luUmVzcG9uc2USPAoJY2F0YWxvZ3VlG\
    AEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSJJCglDYXRhbG9ndWUSPAoJY2F0YWxv\
    Z3VlGAEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSLlAQoEQm9keRIgCgtjb250ZW5\
    0VHlwZRgBIAEoCVILY29udGVudFR5cGUSNQoHY29udGVudBgCIAEoCzIbLmdvb2dsZS5wcm90b2J1Zi5CeXRlc1ZhbHVlUg\
    djb250ZW50Ek4KD2NvbnRlbnRUeXBlSGludBgDIAEoDjIkLmlvLnBhY3QucGx1Z2luLkJvZHkuQ29udGVudFR5cGVIaW50U\
    g9jb250ZW50VHlwZUhpbnQiNAoPQ29udGVudFR5cGVIaW50EgsKB0RFRkFVTFQQABIICgRURVhUEAESCgoGQklOQVJZEAIi\
    pQMKFkNvbXBhcmVDb250ZW50c1JlcXVlc3QSMAoIZXhwZWN0ZWQYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5UghleHB\
    lY3RlZBIsCgZhY3R1YWwYAiABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5UgZhY3R1YWwSMgoVYWxsb3dfdW5leHBlY3RlZF\
    9rZXlzGAMgASgIUhNhbGxvd1VuZXhwZWN0ZWRLZXlzEkcKBXJ1bGVzGAQgAygLMjEuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZ\
    UNvbnRlbnRzUmVxdWVzdC5SdWxlc0VudHJ5UgVydWxlcxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFj\
    dC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbhpXCgpSdWxlc0VudHJ5EhAKA2tleRg\
    BIAEoCVIDa2V5EjMKBXZhbHVlGAIgASgLMh0uaW8ucGFjdC5wbHVnaW4uTWF0Y2hpbmdSdWxlc1IFdmFsdWU6AjgBIkkKE0\
    NvbnRlbnRUeXBlTWlzbWF0Y2gSGgoIZXhwZWN0ZWQYASABKAlSCGV4cGVjdGVkEhYKBmFjdHVhbBgCIAEoCVIGYWN0dWFsI\
    sMBCg9Db250ZW50TWlzbWF0Y2gSNwoIZXhwZWN0ZWQYASABKAsyGy5nb29nbGUucHJvdG9idWYuQnl0ZXNWYWx1ZVIIZXhw\
    ZWN0ZWQSMwoGYWN0dWFsGAIgASgLMhsuZ29vZ2xlLnByb3RvYnVmLkJ5dGVzVmFsdWVSBmFjdHVhbBIaCghtaXNtYXRjaBg\
    DIAEoCVIIbWlzbWF0Y2gSEgoEcGF0aBgEIAEoCVIEcGF0aBISCgRkaWZmGAUgASgJUgRkaWZmIlQKEUNvbnRlbnRNaXNtYX\
    RjaGVzEj8KCm1pc21hdGNoZXMYASADKAsyHy5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hSCm1pc21hdGNoZXMip\
    wIKF0NvbXBhcmVDb250ZW50c1Jlc3BvbnNlEhQKBWVycm9yGAEgASgJUgVlcnJvchJHCgx0eXBlTWlzbWF0Y2gYAiABKAsy\
    Iy5pby5wYWN0LnBsdWdpbi5Db250ZW50VHlwZU1pc21hdGNoUgx0eXBlTWlzbWF0Y2gSTgoHcmVzdWx0cxgDIAMoCzI0Lml\
    vLnBhY3QucGx1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlLlJlc3VsdHNFbnRyeVIHcmVzdWx0cxpdCgxSZXN1bHRzRW\
    50cnkSEAoDa2V5GAEgASgJUgNrZXkSNwoFdmFsdWUYAiABKAsyIS5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hlc\
    1IFdmFsdWU6AjgBIoABChtDb25maWd1cmVJbnRlcmFjdGlvblJlcXVlc3QSIAoLY29udGVudFR5cGUYASABKAlSC2NvbnRl\
    bnRUeXBlEj8KDmNvbnRlbnRzQ29uZmlnGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIOY29udGVudHNDb25maWc\
    iUwoMTWF0Y2hpbmdSdWxlEhIKBHR5cGUYASABKAlSBHR5cGUSLwoGdmFsdWVzGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLl\
    N0cnVjdFIGdmFsdWVzIkEKDU1hdGNoaW5nUnVsZXMSMAoEcnVsZRgBIAMoCzIcLmlvLnBhY3QucGx1Z2luLk1hdGNoaW5nU\
    nVsZVIEcnVsZSJQCglHZW5lcmF0b3ISEgoEdHlwZRgBIAEoCVIEdHlwZRIvCgZ2YWx1ZXMYAiABKAsyFy5nb29nbGUucHJv\
    dG9idWYuU3RydWN0UgZ2YWx1ZXMisQEKE1BsdWdpbkNvbmZpZ3VyYXRpb24SUwoYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9\
    uGAEgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9uEkUKEXBhY3RDb25maW\
    d1cmF0aW9uGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIRcGFjdENvbmZpZ3VyYXRpb24iiAYKE0ludGVyYWN0a\
    W9uUmVzcG9uc2USMAoIY29udGVudHMYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5Ughjb250ZW50cxJECgVydWxlcxgC\
    IAMoCzIuLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuUnVsZXNFbnRyeVIFcnVsZXMSUwoKZ2VuZXJhdG9\
    ycxgDIAMoCzIzLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuR2VuZXJhdG9yc0VudHJ5UgpnZW5lcmF0b3\
    JzEkEKD21lc3NhZ2VNZXRhZGF0YRgEIAEoCzIXLmdvb2dsZS5wcm90b2J1Zi5TdHJ1Y3RSD21lc3NhZ2VNZXRhZGF0YRJVC\
    hNwbHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2lu\
    Q29uZmlndXJhdGlvbhIsChFpbnRlcmFjdGlvbk1hcmt1cBgGIAEoCVIRaW50ZXJhY3Rpb25NYXJrdXASZAoVaW50ZXJhY3R\
    pb25NYXJrdXBUeXBlGAcgASgOMi4uaW8ucGFjdC5wbHVnaW4uSW50ZXJhY3Rpb25SZXNwb25zZS5NYXJrdXBUeXBlUhVpbn\
    RlcmFjdGlvbk1hcmt1cFR5cGUSGgoIcGFydE5hbWUYCCABKAlSCHBhcnROYW1lGlcKClJ1bGVzRW50cnkSEAoDa2V5GAEgA\
    SgJUgNrZXkSMwoFdmFsdWUYAiABKAsyHS5pby5wYWN0LnBsdWdpbi5NYXRjaGluZ1J1bGVzUgV2YWx1ZToCOAEaWAoPR2Vu\
    ZXJhdG9yc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5Ei8KBXZhbHVlGAIgASgLMhkuaW8ucGFjdC5wbHVnaW4uR2VuZXJhdG9\
    yUgV2YWx1ZToCOAEiJwoKTWFya3VwVHlwZRIPCgtDT01NT05fTUFSSxAAEggKBEhUTUwQASLSAQocQ29uZmlndXJlSW50ZX\
    JhY3Rpb25SZXNwb25zZRIUCgVlcnJvchgBIAEoCVIFZXJyb3ISRQoLaW50ZXJhY3Rpb24YAiADKAsyIy5pby5wYWN0LnBsd\
    Wdpbi5JbnRlcmFjdGlvblJlc3BvbnNlUgtpbnRlcmFjdGlvbhJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8u\
    cGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbiLTAgoWR2VuZXJhdGVDb250ZW5\
    0UmVxdWVzdBIwCghjb250ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlbnRzElYKCmdlbmVyYXRvcn\
    MYAiADKAsyNi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0LkdlbmVyYXRvcnNFbnRyeVIKZ2VuZXJhd\
    G9ycxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblIT\
    cGx1Z2luQ29uZmlndXJhdGlvbhpYCg9HZW5lcmF0b3JzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLwoFdmFsdWUYAiABKAs\
    yGS5pby5wYWN0LnBsdWdpbi5HZW5lcmF0b3JSBXZhbHVlOgI4ASJLChdHZW5lcmF0ZUNvbnRlbnRSZXNwb25zZRIwCghjb2\
    50ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlbnRzMuIDCgpQYWN0UGx1Z2luElMKCkluaXRQbHVna\
    W4SIS5pby5wYWN0LnBsdWdpbi5Jbml0UGx1Z2luUmVxdWVzdBoiLmlvLnBhY3QucGx1Z2luLkluaXRQbHVnaW5SZXNwb25z\
    ZRJECg9VcGRhdGVDYXRhbG9ndWUSGS5pby5wYWN0LnBsdWdpbi5DYXRhbG9ndWUaFi5nb29nbGUucHJvdG9idWYuRW1wdHk\
    SYgoPQ29tcGFyZUNvbnRlbnRzEiYuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZUNvbnRlbnRzUmVxdWVzdBonLmlvLnBhY3QucG\
    x1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlEnEKFENvbmZpZ3VyZUludGVyYWN0aW9uEisuaW8ucGFjdC5wbHVnaW4uQ\
    29uZmlndXJlSW50ZXJhY3Rpb25SZXF1ZXN0GiwuaW8ucGFjdC5wbHVnaW4uQ29uZmlndXJlSW50ZXJhY3Rpb25SZXNwb25z\
    ZRJiCg9HZW5lcmF0ZUNvbnRlbnQSJi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0GicuaW8ucGFjdC5\
    wbHVnaW4uR2VuZXJhdGVDb250ZW50UmVzcG9uc2VCEFoOaW8ucGFjdC5wbHVnaW5iBnByb3RvMw==";

  #[test]
  fn find_message_type_by_name_test() {
    let bytes = base64::decode(DESCRIPTORS).unwrap();
    let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
    let fds = FileDescriptorSet::decode(bytes1).unwrap();

    expect!(find_message_type_by_name("", &fds)).to(be_err());
    expect!(find_message_type_by_name("Does not exist", &fds)).to(be_err());

    let result = find_message_type_by_name("GenerateContentRequest", &fds).unwrap();
    expect!(result.name).to(be_some().value("GenerateContentRequest"));
  }

  #[test]
  fn find_nested_type_test() {
    let non_message_field = FieldDescriptorProto {
      r#type: Some(Type::Bytes as i32),
      .. FieldDescriptorProto::default()
    };
    let field_with_no_type_name = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      .. FieldDescriptorProto::default()
    };
    let field_with_incorrect_type_name = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      type_name: Some("field_with_incorrect_type_name".to_string()),
      .. FieldDescriptorProto::default()
    };
    let field_with_matching_type_name = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      type_name: Some("CorrectType".to_string()),
      .. FieldDescriptorProto::default()
    };
    let nested = DescriptorProto {
      name: Some("CorrectType".to_string()),
      .. DescriptorProto::default()
    };
    let message = DescriptorProto {
      field: vec![
        non_message_field.clone(),
        field_with_no_type_name.clone(),
        field_with_incorrect_type_name.clone()
      ],
      nested_type: vec![
        nested.clone()
      ],
      .. DescriptorProto::default()
    };
    expect!(find_nested_type(&message, &non_message_field)).to(be_none());
    expect!(find_nested_type(&message, &field_with_no_type_name)).to(be_none());
    expect!(find_nested_type(&message, &field_with_incorrect_type_name)).to(be_none());
    expect!(find_nested_type(&message, &field_with_matching_type_name)).to(be_some().value(nested));
  }

  #[test]
  fn is_map_field_test() {
    let non_message_field = FieldDescriptorProto {
      r#type: Some(Type::Bytes as i32),
      .. FieldDescriptorProto::default()
    };
    let non_repeated_field = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      .. FieldDescriptorProto::default()
    };
    let repeated_field_with_no_nested_type = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      label: Some(Label::Repeated as i32),
      type_name: Some("field_with_incorrect_type_name".to_string()),
      .. FieldDescriptorProto::default()
    };
    let field_with_non_map_nested_type = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      label: Some(Label::Repeated as i32),
      type_name: Some("NonMapType".to_string()),
      .. FieldDescriptorProto::default()
    };
    let field_with_map_nested_type = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      label: Some(Label::Repeated as i32),
      type_name: Some("MapType".to_string()),
      .. FieldDescriptorProto::default()
    };
    let non_map_nested = DescriptorProto {
      name: Some("NonMapType".to_string()),
      .. DescriptorProto::default()
    };
    let map_nested = DescriptorProto {
      name: Some("MapType".to_string()),
      options: Some(MessageOptions {
        message_set_wire_format: None,
        no_standard_descriptor_accessor: None,
        deprecated: None,
        map_entry: Some(true),
        uninterpreted_option: vec![]
      }),
      .. DescriptorProto::default()
    };
    let message = DescriptorProto {
      field: vec![
        non_message_field.clone(),
        non_repeated_field.clone(),
        repeated_field_with_no_nested_type.clone(),
        field_with_non_map_nested_type.clone(),
        field_with_map_nested_type.clone()
      ],
      nested_type: vec![
        non_map_nested,
        map_nested
      ],
      .. DescriptorProto::default()
    };
    expect!(is_map_field(&message, &non_message_field)).to(be_false());
    expect!(is_map_field(&message, &non_repeated_field)).to(be_false());
    expect!(is_map_field(&message, &repeated_field_with_no_nested_type)).to(be_false());
    expect!(is_map_field(&message, &field_with_non_map_nested_type)).to(be_false());
    expect!(is_map_field(&message, &field_with_map_nested_type)).to(be_true());
  }

  #[test]
  fn as_hex_test() {
    expect!(as_hex(&[])).to(be_equal_to(""));
    expect!(as_hex(&[1, 2, 3, 255])).to(be_equal_to("010203ff"));
  }
}
