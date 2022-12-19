use std::collections::HashMap;

use anyhow::anyhow;
use chrono::{DateTime, Local};
use pact_models::generators::{generate_decimal, GenerateValue, Generator, VariantMatcher};
use pact_models::generators::datetime_expressions::{
  execute_date_expression,
  execute_datetime_expression,
  execute_time_expression
};
use pact_models::json_utils::json_to_string;
use pact_models::time_utils::{parse_pattern, to_chrono_pattern};
use serde_json::Value;
use tracing::{debug, warn};

use crate::message_decoder::ProtobufFieldData;

impl GenerateValue<ProtobufFieldData> for Generator {
  fn generate_value(&self,
                    value: &ProtobufFieldData,
                    context: &HashMap<&str, Value>,
                    matcher: &Box<dyn VariantMatcher + Send + Sync>
  ) -> anyhow::Result<ProtobufFieldData> {
    let result = match self {
      Generator::RandomInt(min, max) => {
        // let rand_int = rand::thread_rng().gen_range(*min..max.saturating_add(1));
        match value {
          // Value::String(_) => Ok(json!(format!("{}", rand_int))),
          // Value::Number(_) => Ok(json!(rand_int)),
          _ => Err(anyhow!("Could not generate a random int from {}", value))
        }
      },
      // Generator::Uuid(format) => match value {
      //   Value::String(_) => match format.unwrap_or_default() {
      //     UuidFormat::Simple => Ok(json!(Uuid::new_v4().as_simple().to_string())),
      //     UuidFormat::LowerCaseHyphenated => Ok(json!(Uuid::new_v4().as_hyphenated().to_string())),
      //     UuidFormat::UpperCaseHyphenated => Ok(json!(Uuid::new_v4().as_hyphenated().to_string().to_uppercase())),
      //     UuidFormat::Urn => Ok(json!(Uuid::new_v4().as_urn().to_string()))
      //   },
      //   _ => Err(anyhow!("Could not generate a UUID from {}", value))
      // },
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
      // Generator::RandomHexadecimal(digits) => match value {
      //   Value::String(_) => Ok(json!(generate_hexadecimal(*digits as usize))),
      //   _ => Err(anyhow!("Could not generate a random hexadecimal from {}", value))
      // },
      // Generator::RandomString(size) => match value {
      //   Value::String(_) => Ok(json!(generate_ascii_string(*size as usize))),
      //   _ => Err(anyhow!("Could not generate a random string from {}", value))
      // },
      // Generator::Regex(ref regex) => {
      //   let mut parser = regex_syntax::ParserBuilder::new().unicode(false).build();
      //   match parser.parse(regex) {
      //     Ok(hir) => {
      //       let gen = rand_regex::Regex::with_hir(hir, 20).unwrap();
      //       Ok(json!(rand::thread_rng().sample::<String, _>(gen)))
      //     },
      //     Err(err) => {
      //       warn!("'{}' is not a valid regular expression - {}", regex, err);
      //       Err(anyhow!("Could not generate a random string from {} - {}", regex, err))
      //     }
      //   }
      // },
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
      // Generator::RandomBoolean => Ok(json!(rand::thread_rng().gen::<bool>())),
      // Generator::ProviderStateGenerator(ref exp, ref dt) =>
      //   match generate_value_from_context(exp, context, dt) {
      //     Ok(val) => val.as_json(),
      //     Err(err) => Err(err)
      //   },
      // Generator::MockServerURL(example, regex) => {
      //   debug!("context = {:?}", context);
      //   if let Some(mock_server_details) = context.get("mockServer") {
      //     match mock_server_details.as_object() {
      //       Some(mock_server_details) => {
      //         match get_field_as_string("href", mock_server_details) {
      //           Some(url) => match Regex::new(regex) {
      //             Ok(re) => Ok(Value::String(replace_with_regex(example, url, re))),
      //             Err(err) => Err(anyhow!("MockServerURL: Failed to generate value: {}", err))
      //           },
      //           None => Err(anyhow!("MockServerURL: can not generate a value as there is no mock server URL in the test context"))
      //         }
      //       },
      //       None => Err(anyhow!("MockServerURL: can not generate a value as the mock server details in the test context is not an Object"))
      //     }
      //   } else {
      //     Err(anyhow!("MockServerURL: can not generate a value as there is no mock server details in the test context"))
      //   }
      // }
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

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::hashmap;
  use pact_matching::generators::DefaultVariantMatcher;
  use pact_models::generators::{GenerateValue, Generator, VariantMatcher};
  use prost_types::{DescriptorProto, EnumDescriptorProto};

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
}
