extern crate core;

pub mod server;
mod protoc;
mod protobuf;
mod message_builder;
pub mod message_decoder;
pub mod utils;
pub mod matching;
pub mod mock_server;
pub mod tcp;
pub mod dynamic_message;
mod mock_service;
mod verification;
mod metadata;

pub mod built_info {
  include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
