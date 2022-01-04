<a href="https://pactflow.io"><img src="docs/pactflow-logo-s.png" alt="pactflow logo" height="60px" align="right"></a>

# Pact Protobuf Plugin [![Pact-Protobuf-Plugin Build](https://github.com/pactflow/pact-protobuf-plugin/actions/workflows/build.yml/badge.svg)](https://github.com/pactflow/pact-protobuf-plugin/actions/workflows/build.yml)

> Pact plugin for testing Protobufs and gRPC services with [Pact](https://docs.pact.io)

## About this plugin

This plugin provides support for matching and verifying Protobuf messages and gRPC service calls. It fits into the
Pact testing framework and extends Pact testing for Protobuf payloads. 

## Table of Content

- [Requirements to use it](#requirements-to-use-it)
- [Installation](#installation)
- [Supported features](#supported-features)
- [Unsupported features](#unsupported-features)
- [Using the plugin](#using-the-plugin)
    - [Testing an interaction with a single Protobuf message](#testing-an-interaction-with-a-single-protobuf-message)
    - [Testing a gRPC service interaction](#testing-a-g-rpc-service-interaction)
- [Support](#support)
- [Contributing to the plugin](#contributing)
- [Development Roadmap](#development-roadmap)

## Requirements to use it

## Installation 

## Supported features

## Unsupported features

## Using the plugin

### Testing an interaction with a single Protobuf message

### Testing a gRPC service interaction

## Support

Join us on slack [![slack](https://slack.pact.io/badge.svg)](https://slack.pact.io) in the **#protobufs** channel

or

    Twitter: @pact_up
    Stack Overflow: stackoverflow.com/questions/tagged/pact


## Contributing

PRs are always welcome!

### Raising defects

Before raising an issue, make sure you have checked the open and closed issues to see if an answer is provided there.
There may also be an answer to your question on [stackoverflow](https://stackoverflow.com/questions/tagged/pact).

Please provide the following information with your issue to enable us to respond as quickly as possible.

1. The relevant versions of the packages you are using (plugin and Pact versions).
1. The steps to recreate your issue.
1. An executable code example where possible.

### New features / changes

1. Fork it
1. Create your feature branch (git checkout -b my-new-feature)
1. Commit your changes (git commit -am 'feat: Add some feature')
1. Push to the branch (git push origin my-new-feature)
1. Create new Pull Request

#### Commit messages

We follow the [Conventional Changelog](https://github.com/bcoe/conventional-changelog-standard/blob/master/convention.md)
message conventions. Please ensure you follow the guidelines.

### Building the plugin

To build the plugin, you need a working Rust environment. Refer to the [Rust Guide](https://www.rust-lang.org/learn/get-started).

The build tool used is `cargo` and you can build the plugin by running `cargo build`. This will compile the plugin and 
put the generated files in `target/debug`. The main plugin executable is `pact-protobuf-plugin`
and this will need to be copied into the Pact plugin directory. See the installation instructions above.

### Running the tests

You can run all the unit tests by executing `cargo test --lib`.

## Development Roadmap

## License and Copyright

This plugin is released under the **MIT License** and is copyright © 2021-22 [Pactflow](https://pactflow.io).

The Pactflow logos are copyright © [Pactflow](https://pactflow.io) and may not be used without permission.
