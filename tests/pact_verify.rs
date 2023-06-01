use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use expectest::prelude::*;
use maplit::hashmap;
use pact_models::http_utils::HttpAuth;
use pact_models::prelude::ProviderState;
use pact_models::v4::message_parts::MessageContents;
use pact_plugin_driver::plugin_manager::shutdown_plugins;
use pact_plugin_driver::proto::InitPluginRequest;
use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
use pact_verifier::{
  FilterInfo,
  NullRequestFilterExecutor,
  PactSource,
  ProviderInfo,
  PublishOptions,
  VerificationOptions,
  verify_provider_async
};
use pact_verifier::callback_executors::ProviderStateExecutor;
use prost::bytes::BytesMut;
use prost::Message;
use reqwest::Client;
use rocket::{Data, Request};
use rocket::data::{ByteUnit, FromData, Outcome};
use rocket::http::{ContentType, Status};
use rocket::outcome::Outcome::{Failure, Success};
use serde_json::Value;
use test_log::test;
use tracing::{debug, error};

use pact_protobuf_plugin::built_info;
use pact_protobuf_plugin::server::ProtobufPactPlugin;

// We are not using provider states, so we define a executor that does nothing
#[derive(Debug)]
struct NoopProviderStateExecutor { }

#[async_trait]
impl ProviderStateExecutor for NoopProviderStateExecutor {
  async fn call(
    self: Arc<Self>,
    _interaction_id: Option<String>,
    _provider_state: &ProviderState,
    _setup: bool,
    _client: Option<&Client>
  ) -> anyhow::Result<HashMap<String, Value>> {
    Ok(hashmap!{})
  }

  fn teardown(self: &Self) -> bool {
    false
  }
}

// Structure to hold the data we receive from the Pact verifier
#[derive(Debug)]
struct MessageRequest {
  pub contents: MessageContents
}

// We can't use the default Rocket serialisation because we want to use the Pact models and they
// don't implement the serde serialisation traits. Not too hard to do it ourselves using the
// FromData trait
#[rocket::async_trait]
impl <'r> FromData<'r> for MessageRequest {
  type Error = anyhow::Error;

  async fn from_data(req: &'r Request<'_>, data: Data<'r>) -> Outcome<'r, Self> {
    if let Some(content_type) = req.content_type() {
      if content_type.is_json() {
        match data.open(ByteUnit::max_value()).into_bytes().await {
          Ok(data) => match serde_json::from_slice::<Value>(data.value.as_slice()) {
            Ok(json) => {
              if let Some(contents) = json.get("request") {
                debug!("contents = {:?}", contents);
                match MessageContents::from_json(contents) {
                  Ok(contents) => Success(MessageRequest { contents }),
                  Err(err) => Failure((Status::UnprocessableEntity, anyhow!(err)))
                }
              } else {
                Failure((Status::UnprocessableEntity, anyhow!("Missing request")))
              }
            }
            Err(err) => Failure((Status::UnprocessableEntity, anyhow!("Failed to parse JSON: {}", err)))
          }
          Err(err) => Failure((Status::UnprocessableEntity, anyhow!(err)))
        }
      } else {
        Failure((Status::UnprocessableEntity, anyhow!("Expected JSON, got {}", content_type)))
      }
    } else {
      Failure((Status::UnprocessableEntity, anyhow!("No content type")))
    }
  }
}

// Rocket server used to pass the Protobuf messages from/to the verifier. Each request should
// contain the content type which will have a message attribute telling us which message it is
// for.
#[rocket::post("/", data = "<request>")]
async fn messages(request: MessageRequest) -> (Status, (ContentType, Vec<u8>)) {
  debug!("Got request = {:?}", request);
  if let Some(content_type) = request.contents.message_content_type() {
    if content_type.sub_type == "protobuf" {
      if let Some(message_type) = content_type.attributes.get("message") {
        match message_type.as_str() {
          "InitPluginRequest" => init_plugin_request(&request).await,
          _ => (Status::BadRequest, (ContentType::Text, Vec::from("Unknown protobuf message type provided")))
        }
      } else {
        (Status::BadRequest, (ContentType::Text, Vec::from("Not a protobuf message type provided")))
      }
    } else {
      (Status::BadRequest, (ContentType::Text, Vec::from("Not a protobuf message")))
    }
  } else {
    (Status::BadRequest, (ContentType::Text, Vec::from("No content type for the provided message")))
  }
}

// Handle the init plugin request message and return the response. We do this by calling the actual
// plugin server init_plugin method, but pass in the Protobuf message we get from the verifier.
async fn init_plugin_request(request_message: &MessageRequest) -> (Status, (ContentType, Vec<u8>)) {
  if let Some(data) = request_message.contents.contents.value() {
    match InitPluginRequest::decode(data) {
      Ok(request) => {
        debug!("Got init plugin request {:?}", request);

        // This is were we call our actual service method, passing in the input message
        // and getting the output message as the response, which we then return in encoded form
        let plugin = ProtobufPactPlugin::new();
        match plugin.init_plugin(tonic::Request::new(request)).await {
          Ok(response) => {
            debug!("Got init plugin response {:?}", response);
            let mut buffer = BytesMut::new();
            match response.get_ref().encode(&mut buffer) {
              Ok(_) => {
                (Status::Ok, (ContentType::new("application", "protobuf").with_params(("message", "InitPluginResponse")),
                              buffer.to_vec()))
              }
              Err(err) => {
                error!("Failed to write response to buffer - {}", err);
                (Status::BadRequest, (ContentType::Text, Vec::from("Failed to write response to buffer")))
              }
            }
          }
          Err(err) => {
            error!("Failed to generate response for InitPluginRequest - {}", err);
            (Status::BadRequest, (ContentType::Text, Vec::from("Failed to generate response message")))
          }
        }
      }
      Err(err) => {
        error!("Failed to parse request message - {}", err);
        (Status::BadRequest, (ContentType::Text, Vec::from("Failed to parse request message")))
      }
    }
  } else {
    (Status::BadRequest, (ContentType::Text, Vec::from("Request did not contain any data")))
  }
}

// Pact verification test. This first starts up a Rocket server that can provide the Protobuf
// messages required by the Pact verifier.
#[test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
async fn verify_plugin() {
  // Test Setup
  #[allow(deprecated)]
  let provider_info = ProviderInfo {
    name: "plugin".to_string(),
    port: Some(8000),
    .. ProviderInfo::default()
  };
  // Set the source to fetch from the Pact broker
  let source = PactSource::BrokerWithDynamicConfiguration {
    provider_name: "plugin".to_string(),
    broker_url: "https://pact-foundation.pactflow.io".to_string(),
    enable_pending: false,
    include_wip_pacts_since: None,
    provider_tags: vec![],
    provider_branch: None,
    selectors: vec![],
    auth: Some(HttpAuth::Token(env::var("PACTFLOW_TOKEN")
      .expect("The PACTFLOW_TOKEN environment variable must be set").to_string())),
    links: vec![]
  };
  // Set the version to be the plugin name + version + git SHA
  let mut version = "pact-protobuf-plugin:".to_string();
  version.push_str(env!("CARGO_PKG_VERSION"));
  version.push_str("+");
  version.push_str(built_info::GIT_COMMIT_HASH.unwrap_or("0"));

  let options: VerificationOptions<NullRequestFilterExecutor> = VerificationOptions::default();
  let publish_options = if env::var("CI").map(|v| v == "true").unwrap_or(false)  {
    Some(PublishOptions {
      provider_version: Some(version),
      build_url: None,
      provider_tags: vec!["pact-protobuf-plugin".to_string()],
      provider_branch: None
    })
  } else {
    None
  };
  let ps_executor = NoopProviderStateExecutor {};

  // Start the rocket server
  let server = rocket::build()
    .mount("/", rocket::routes![messages])
    .ignite()
    .await.expect("Could not start the Rocket server");
  let shutdown = server.shutdown();
  tokio::spawn(server.launch());

  // Execute the verification
  let result = verify_provider_async(
    provider_info,
    vec![source],
    FilterInfo::None,
    vec![],
    &options,
    publish_options.as_ref(),
    &Arc::new(ps_executor), None
  ).await;

  // Need to shutdown all the things, otherwise we could leave hanging plugin processes.
  shutdown.notify();
  shutdown_plugins();

  // Confirm that the verification was successful
  expect!(result.unwrap().result).to(be_true());
}
