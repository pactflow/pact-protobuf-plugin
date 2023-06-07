use std::collections::HashMap;

use anyhow::anyhow;
use chrono::{DateTime, Local};
use pact_models::generators::{
  generate_ascii_string,
  generate_decimal,
  generate_hexadecimal,
  generate_value_from_context,
  GenerateValue,
  Generator,
  UuidFormat,
  VariantMatcher
};
use pact_models::generators::datetime_expressions::{
  execute_date_expression,
  execute_datetime_expression,
  execute_time_expression
};
use pact_models::json_utils::{get_field_as_string, json_to_string};
use pact_models::time_utils::{parse_pattern, to_chrono_pattern};
use rand::prelude::*;
use regex::{Captures, Regex};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::message_decoder::ProtobufFieldData;

impl GenerateValue<ProtobufFieldData> for Generator {
  fn generate_value(&self,
                    value: &ProtobufFieldData,
                    context: &HashMap<&str, Value>,
                    matcher: &Box<dyn VariantMatcher + Send + Sync>
  ) -> anyhow::Result<ProtobufFieldData> {
    let result = match self {
      Generator::RandomInt(min, max) => {
        let rand_int = thread_rng().gen_range(*min..max.saturating_add(1));
        match value {
          ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(rand_int.to_string())),
          ProtobufFieldData::Double(_) => Ok(ProtobufFieldData::Double(rand_int as f64)),
          ProtobufFieldData::Float(_) => Ok(ProtobufFieldData::Float(rand_int as f32)),
          ProtobufFieldData::Integer64(_) => Ok(ProtobufFieldData::Integer64(rand_int as i64)),
          ProtobufFieldData::Integer32(_) => Ok(ProtobufFieldData::Integer32(rand_int)),
          ProtobufFieldData::UInteger64(_) => Ok(ProtobufFieldData::UInteger64(rand_int as u64)),
          ProtobufFieldData::UInteger32(_) => Ok(ProtobufFieldData::UInteger32(rand_int as u32)),
          _ => Err(anyhow!("Could not generate a random int from {}", value))
        }
      },
      Generator::Uuid(format) => match value {
        ProtobufFieldData::String(_) => match format.unwrap_or_default() {
          UuidFormat::Simple => Ok(ProtobufFieldData::String(Uuid::new_v4().as_simple().to_string())),
          UuidFormat::LowerCaseHyphenated => Ok(ProtobufFieldData::String(Uuid::new_v4().as_hyphenated().to_string())),
          UuidFormat::UpperCaseHyphenated => Ok(ProtobufFieldData::String(Uuid::new_v4().as_hyphenated().to_string().to_uppercase())),
          UuidFormat::Urn => Ok(ProtobufFieldData::String(Uuid::new_v4().as_urn().to_string()))
        },
        _ => Err(anyhow!("Could not generate a UUID from {}", value))
      },
      Generator::RandomDecimal(digits) => {
        let decimal = generate_decimal(*digits as usize);
        match value {
          ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(decimal)),
          ProtobufFieldData::Double(_) => Ok(ProtobufFieldData::Double(decimal.parse()?)),
          ProtobufFieldData::Float(_) => Ok(ProtobufFieldData::Float(decimal.parse()?)),
          ProtobufFieldData::Integer64(_) => Ok(ProtobufFieldData::Integer64(decimal.parse()?)),
          ProtobufFieldData::Integer32(_) => Ok(ProtobufFieldData::Integer32(decimal.parse()?)),
          ProtobufFieldData::UInteger64(_) => Ok(ProtobufFieldData::UInteger64(decimal.parse()?)),
          ProtobufFieldData::UInteger32(_) => Ok(ProtobufFieldData::UInteger32(decimal.parse()?)),
          _ => Err(anyhow!("Could not generate a random decimal from {}", value))
        }
      },
      Generator::RandomHexadecimal(digits) => match value {
        ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(generate_hexadecimal(*digits as usize))),
        _ => Err(anyhow!("Could not generate a random hexadecimal from {}", value))
      },
      Generator::RandomString(size) => match value {
        ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(generate_ascii_string(*size as usize))),
        _ => Err(anyhow!("Could not generate a random string from {}", value))
      },
      Generator::Regex(ref regex) => match value {
        ProtobufFieldData::String(_) => {
          let mut parser = regex_syntax::ParserBuilder::new().unicode(false).build();
          match parser.parse(regex) {
            Ok(hir) => {
              let gen = rand_regex::Regex::with_hir(hir, 20).unwrap();
              Ok(ProtobufFieldData::String(rand::thread_rng().sample::<String, _>(gen)))
            },
            Err(err) => {
              warn!("'{}' is not a valid regular expression - {}", regex, err);
              Err(anyhow!("Could not generate a random string from {} - {}", regex, err))
            }
          }
        }
        _ => Err(anyhow!("Could not generate a random regex from {}", value))
      },
      Generator::Date(ref format, exp) => {
        let base = match context.get("baseDate") {
          None => Local::now(),
          Some(d) => json_to_string(d).parse::<DateTime<Local>>()?
        };
        let date = execute_date_expression(&base, exp.clone().unwrap_or_default().as_str())?;
        let result = match format {
          Some(pattern) => match parse_pattern(pattern) {
            Ok(tokens) => {
              #[allow(deprecated)]
              Ok(date.date().format(&to_chrono_pattern(&tokens)).to_string())
            },
            Err(err) => {
              warn!("Date format {} is not valid - {}", pattern, err);
              Err(anyhow!("Could not generate a random date from {} - {}", pattern, err))
            }
          },
          None => Ok(date.naive_local().date().to_string())
        };
        result.and_then(|v| {
          match value {
            ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(v)),
            _ => Err(anyhow!("Can not generate a date value for a field type {:?}", value))
          }
        })
      },
      Generator::Time(ref format, exp) => {
        let base = match context.get("baseTime") {
          None => Local::now(),
          Some(d) => json_to_string(d).parse::<DateTime<Local>>()?
        };
        let time = execute_time_expression(&base, exp.clone().unwrap_or_default().as_str())?;
        let result = match format {
          Some(pattern) => match parse_pattern(pattern) {
            Ok(tokens) => {
              #[allow(deprecated)]
              Ok(time.time().format(&to_chrono_pattern(&tokens)).to_string())
            },
            Err(err) => {
              warn!("Time format {} is not valid - {}", pattern, err);
              Err(anyhow!("Could not generate a random time from {} - {}", pattern, err))
            }
          },
          None => Ok(time.naive_local().time().to_string())
        };
        result.and_then(|v| {
          match value {
            ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(v)),
            _ => Err(anyhow!("Can not generate a time value for a field type {:?}", value))
          }
        })
      },
      Generator::DateTime(ref format, exp) => {
        let base = match context.get("baseDateTime") {
          None => Local::now(),
          Some(d) => json_to_string(d).parse::<DateTime<Local>>()?
        };
        let date_time = execute_datetime_expression(&base, exp.clone().unwrap_or_default().as_str())?;
        let result = match format {
          Some(pattern) => match parse_pattern(pattern) {
            Ok(tokens) => {
              #[allow(deprecated)]
              Ok(date_time.format(&to_chrono_pattern(&tokens)).to_string())
            },
            Err(err) => {
              warn!("Date format {} is not valid - {}", pattern, err);
              Err(anyhow!("Could not generate a random date-time from {} - {}", pattern, err))
            }
          },
          None => Ok(date_time.format("%Y-%m-%dT%H:%M:%S.%3f%z").to_string())
        };
        result.and_then(|v| {
          match value {
            ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(v)),
            _ => Err(anyhow!("Can not generate a date-time value for a field type {:?}", value))
          }
        })
      },
      Generator::RandomBoolean => {
        let b = thread_rng().gen::<bool>();
        match value {
          ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(b.to_string())),
          ProtobufFieldData::Boolean(_) => Ok(ProtobufFieldData::Boolean(b)),
          ProtobufFieldData::UInteger32(_) => Ok(ProtobufFieldData::UInteger32(u32::from(b))),
          ProtobufFieldData::Integer32(_) => Ok(ProtobufFieldData::Integer32(i32::from(b))),
          ProtobufFieldData::UInteger64(_) => Ok(ProtobufFieldData::UInteger64(u64::from(b))),
          ProtobufFieldData::Integer64(_) => Ok(ProtobufFieldData::Integer64(i64::from(b))),
          ProtobufFieldData::Float(_) => Ok(ProtobufFieldData::Float(if b { 1.0 } else { 0.0 })),
          ProtobufFieldData::Double(_) => Ok(ProtobufFieldData::Double(if b { 1.0 } else { 0.0 })),
          _ => Err(anyhow!("Can not generate a boolean value for a field type {:?}", value))
        }
      },
      Generator::ProviderStateGenerator(ref exp, ref dt) =>
        match generate_value_from_context(exp, context, dt) {
          Ok(val) => match value {
            ProtobufFieldData::String(_) => Ok(ProtobufFieldData::String(val.to_string())),
            ProtobufFieldData::Boolean(_) => Ok(ProtobufFieldData::Boolean(bool::try_from(val)?)),
            ProtobufFieldData::UInteger32(_) => Ok(ProtobufFieldData::UInteger32(u64::try_from(val)? as u32)),
            ProtobufFieldData::Integer32(_) => Ok(ProtobufFieldData::Integer32(i64::try_from(val)? as i32)),
            ProtobufFieldData::UInteger64(_) => Ok(ProtobufFieldData::UInteger64(u64::try_from(val)?)),
            ProtobufFieldData::Integer64(_) => Ok(ProtobufFieldData::Integer64(i64::try_from(val)?)),
            ProtobufFieldData::Float(_) => Ok(ProtobufFieldData::Float(f64::try_from(val)? as f32)),
            ProtobufFieldData::Double(_) => Ok(ProtobufFieldData::Double(f64::try_from(val)?)),
            _ => Err(anyhow!("Can not generate a value from the provider state for a field type {:?}", value))
          },
          Err(err) => Err(err)
        },
      Generator::MockServerURL(example, regex) => {
        debug!("context = {:?}", context);
        if let Some(mock_server_details) = context.get("mockServer") {
          match mock_server_details.as_object() {
            Some(mock_server_details) => {
              match get_field_as_string("href", mock_server_details) {
                Some(url) => match Regex::new(regex) {
                  Ok(re) => Ok(ProtobufFieldData::String(replace_with_regex(example, url, re))),
                  Err(err) => Err(anyhow!("MockServerURL: Failed to generate value: {}", err))
                },
                None => Err(anyhow!("MockServerURL: can not generate a value as there is no mock server URL in the test context"))
              }
            },
            None => Err(anyhow!("MockServerURL: can not generate a value as the mock server details in the test context is not an Object"))
          }
        } else {
          Err(anyhow!("MockServerURL: can not generate a value as there is no mock server details in the test context"))
        }
      }
      // TODO: need to implement ArrayContains with Protobuf repeated fields
      // Generator::ArrayContains(variants) => match value {
      //   Value::Array(vec) => {
      //     let mut result = vec.clone();
      //     for (index, value) in vec.iter().enumerate() {
      //       if let Some((variant, generators)) = matcher.find_matching_variant(value, variants) {
      //         debug!("Generating values for variant {} and value {}", variant, value);
      //         let mut handler = JsonHandler { value: value.clone() };
      //         for (key, generator) in generators {
      //           handler.apply_key(&key, &generator, context, matcher);
      //         };
      //         debug!("Generated value {}", handler.value);
      //         result[index] = handler.value.clone();
      //       }
      //     }
      //     Ok(Value::Array(result))
      //   }
      //   _ => Err(anyhow!("can only use ArrayContains with lists"))
      // }
      _ => Err(anyhow!("Generator type {} is not currently supported for {:?}", self.name(), value))
    };
    debug!("Generated value = {:?}", result);
    result
  }
}

fn replace_with_regex(example: &String, url: String, re: Regex) -> String {
  re.replace(example, |caps: &Captures| {
    let m = caps.get(1).unwrap();
    format!("{}{}", url, m.as_str())
  }).to_string()
}

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::hashmap;
  use pact_matching::generators::DefaultVariantMatcher;
  use pact_models::generators::{GenerateValue, Generator, UuidFormat, VariantMatcher};
  use prost_types::{DescriptorProto, EnumDescriptorProto};
  use regex::Regex;
  use serde_json::Value;

  use crate::message_decoder::ProtobufFieldData;

  #[test_log::test]
  fn generate_datetime() {
    let generator = Generator::DateTime(Some("yyyyhh".to_string()), None);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().to_string()).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_date() {
    let generator = Generator::Date(Some("yyyy".to_string()), None);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().to_string()).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_time() {
    let generator = Generator::Time(Some("hh::mm".to_string()), None);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().to_string()).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_decimal() {
    let generator = Generator::RandomDecimal(10);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().to_string()).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_f32().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_f64().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_int() {
    let generator = Generator::RandomInt(1, 50);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_i64().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_u64().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_i32().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_u32().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().to_string()).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_f32().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_f64().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_uuid() {
    let generator = Generator::Uuid(None);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    expect!(s).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let generator = Generator::Uuid(Some(UuidFormat::Urn));
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    let re = Regex::new("urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    expect!(re.is_match(&s)).to(be_true());

    let generator = Generator::Uuid(Some(UuidFormat::LowerCaseHyphenated));
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    let re = Regex::new("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    expect!(re.is_match(&s)).to(be_true());

    let generator = Generator::Uuid(Some(UuidFormat::UpperCaseHyphenated));
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    let re = Regex::new("[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}").unwrap();
    expect!(re.is_match(&s)).to(be_true());
  }

  #[test_log::test]
  fn generate_hexadecimal() {
    let generator = Generator::RandomHexadecimal(10);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm).unwrap();
    let s = result.as_str().unwrap();
    expect!(s).to_not(be_equal_to("100"));
    let re = Regex::new("[0-9A-F]{10}").unwrap();
    expect!(re.is_match(s)).to(be_true());

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_string() {
    let generator = Generator::RandomString(10);
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    expect!(s).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_regex() {
    let generator = Generator::Regex("\\d+".to_string());
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    expect!(s).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_bool() {
    let generator = Generator::RandomBoolean;
    let vm = DefaultVariantMatcher.boxed();

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_i64().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_u64().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_i32().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_u32().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    let s = result.unwrap().to_string();
    expect!(s).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let result = generator.generate_value(&value, &hashmap!{}, &vm).unwrap();
    let s = result.to_string();
    let re = Regex::new("true|false").unwrap();
    expect!(re.is_match(&s)).to(be_true());

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_f32().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result.unwrap().as_f64().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &hashmap!{}, &vm);
    expect!(result).to(be_err());
  }

  #[test_log::test]
  fn generate_provider_state() {
    let generator = Generator::ProviderStateGenerator("a".to_string(), None);
    let vm = DefaultVariantMatcher.boxed();
    let provider_state = hashmap!{
      "a" => Value::String("50".to_string())
    };

    let value = ProtobufFieldData::Integer64(100);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result.unwrap().as_i64().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::UInteger64(100);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result.unwrap().as_u64().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::Integer32(100);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result.unwrap().as_i32().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::UInteger32(100);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result.unwrap().as_u32().unwrap()).to_not(be_equal_to(100));

    let value = ProtobufFieldData::String("100".to_string());
    let result = generator.generate_value(&value, &provider_state, &vm);
    let s = result.unwrap().to_string();
    expect!(s).to_not(be_equal_to("100"));

    let value = ProtobufFieldData::Boolean(true);
    let provider_state2 = hashmap!{
      "a" => Value::Bool(false)
    };
    let result = generator.generate_value(&value, &provider_state2, &vm).unwrap();
    let s = result.to_string();
    expect!(s).to(be_equal_to("false"));

    let value = ProtobufFieldData::Float(100.0);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result.unwrap().as_f32().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Double(100.0);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result.unwrap().as_f64().unwrap()).to_not(be_equal_to(100.0));

    let value = ProtobufFieldData::Bytes(vec![]);
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Enum(1, EnumDescriptorProto {
      name: None,
      value: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result).to(be_err());

    let value = ProtobufFieldData::Message(vec![], DescriptorProto {
      name: None,
      field: vec![],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    });
    let result = generator.generate_value(&value, &provider_state, &vm);
    expect!(result).to(be_err());
  }
}
