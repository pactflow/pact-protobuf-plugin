extern crate core;

pub mod dynamic_message;
pub mod matching;
mod message_builder;
pub mod message_decoder;
mod metadata;
pub mod mock_server;
mod mock_service;
mod protobuf;
mod protoc;
pub mod server;
pub mod tcp;
pub mod utils;
mod verification;

pub mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
