extern crate core;

pub mod server;
mod protoc;
mod protobuf;
mod message_builder;
mod message_decoder;
mod utils;
mod matching;
pub mod mock_server;
pub mod tcp;
mod dynamic_message;
mod mock_service;
mod verification;

pub mod built_info {
  include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
