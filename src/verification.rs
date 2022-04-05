//! Module with all the functions to verify a gRPC interaction

use std::collections::HashMap;

use pact_models::prelude::OptionalBody;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::sync_message::SynchronousMessage;
use pact_plugin_driver::proto;
use pact_verifier::verification_result::VerificationExecutionResult;
use serde_json::Value;
use tracing::{error, instrument, trace};

/// Verify a gRPC interaction
#[instrument]
pub fn verify_interaction(
  pact: &V4Pact,
  interaction: &SynchronousMessage,
  request_body: &OptionalBody,
  metadata: &HashMap<String, proto::MetadataValue>,
  config: &HashMap<String, Value>
) -> VerificationExecutionResult {
  todo!()
}
