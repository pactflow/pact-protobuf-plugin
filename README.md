<a href="https://pactflow.io"><img src="docs/pactflow-logo-s.png" alt="pactflow logo" height="60px" align="right"></a>

# Pact Protobuf Plugin [![Pact-Protobuf-Plugin Build](https://github.com/pactflow/pact-protobuf-plugin/actions/workflows/build.yml/badge.svg)](https://github.com/pactflow/pact-protobuf-plugin/actions/workflows/build.yml)

> Pact plugin for testing messages and gRPC service calls encoded with as [Protocol buffers](https://developers.google.com/protocol-buffers)
> using the [Pact](https://docs.pact.io) contract testing framework.

## About this plugin

This plugin provides support for matching and verifying Protobuf messages and gRPC service calls. It fits into the
[Pact contract testing framework](https://docs.pact.io) and extends Pact testing for [Protocol buffer](https://developers.google.com/protocol-buffers) payloads. 

## Table of Content

- [Requirements to use it](#requirements-to-use-it)
- [Installation](#installation)
  - [Installing the plugin](#installing-the-plugin)
  - [Installing the Protocol buffer protoc compiler](#installing-the-protocol-buffer-protoc-compiler) 
- [Supported features](#supported-features)
- [Unsupported features](#unsupported-features)
- [Using the plugin](#using-the-plugin)
    - [Testing an interaction with a single Protobuf message](#testing-an-interaction-with-a-single-protobuf-message)
    - [Testing a gRPC service interaction](#testing-a-g-rpc-service-interaction)
- [Support](#support)
- [Contributing to the plugin](#contributing)
- [Development Roadmap](#development-roadmap)

## Requirements to use it

This plugin provides matching and verification of Protobuf proto3 encoded messages to the Pact contract testing framework. It requires a version
of the Pact framework that supports the [V4 Pact specification](https://github.com/pact-foundation/pact-specification/tree/version-4) 
as well as the [Pact plugin framework](https://github.com/pact-foundation/pact-plugins).

Supported Pact versions:
- [Pact-JVM v4.2.x](https://github.com/pact-foundation/pact-jvm)
- [Pact-Rust Consumer v0.8.x](https://github.com/pact-foundation/pact-reference/tree/master/rust/pact_consumer)
- [Pact-Rust Verifier v0.12.x](https://github.com/pact-foundation/pact-reference/tree/master/rust/pact_verifier_cli)

To support compiling Protocol Buffer proto files requires a version of the [Protocol Buffer compiler](https://github.com/protocolbuffers/protobuf).

## Installation

The executable binaries and plugin manifest file for the plugin can be downloaded from the project [releases page](../releases). There will be an executable for each
operating system and architecture. If your particular operating system or architecture is not supported, please send
a request to [support@pactflow.io](support@pactflow.io) with the details.

### Installing the plugin
To install the plugin requires the plugin executable binary as well as the plugin manifest file to be unpacked/copied into
a Pact plugin directory. By default, this will be `.pact/plugins/protobuf-<version>` in the home directory (i.e. `$HOME/.pact/plugins/protobuf-0.0.0`).

Example installation of Linux version 0.0.0: 
1. Create the plugin directory if needed: `mkdir -p ~/.pact/plugins/protobuf-0.0.0`
2. Download the plugin manifest into the directory: `wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-0.0.0/pact-plugin.json -O ~/.pact/plugins/protobuf-0.0.0/pact-plugin.json`
3. Download the plugin executable into the directory: `wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-0.0.0/pact-protobuf-plugin-linux-x86_64.gz  -O ~/.pact/plugins/protobuf-0.0.0/pact-protobuf-plugin-linux-x86_64.gz`
4. Unpack the plugin executable: `gunzip -N ~/.pact/plugins/protobuf-0.0.0/pact-protobuf-plugin-linux-x86_64.gz`

**Note:** The unpacked executable name must match the `entryPoint` value in the manifest file. By default this is
`pact-protobuf-plugin` on unix* and `pact-protobuf-plugin.exe` on Windows.

#### Overriding the default Pact plugin directory

The default plugin directory (`$HOME/.pact/plugins`) can be changed by setting the `PACT_PLUGIN_DIR` environment variable.

### Installing the Protocol buffer protoc compiler

The plugin can automatically download the correct version of the Protocol buffer compiler for the current operating system
and architecture. By default, it will download the compiler from https://github.com/protocolbuffers/protobuf/releases
and then unpack it into the plugin's installation directory.

The plugin executes the following steps:

1. Look for a valid `protoc/bin/protoc` in the plugin installation directory
2. If not found, look for a `protoc-{version}-{OS}.zip` in the plugin installation directory and unpack that (i.e. for Linux it will look for `protoc-3.19.1-linux-x86_64.zip`).
3. If not found, try download protoc using the `downloadUrl` entry in the plugin manifest file
4. Otherwise, fallback to using the system installed protoc

#### Dealing with network and firewall issues

If the plugin is going to run in an environment that does not allow automatic downloading of files, then you can do any of the following:

1. Download the protoc archive and place it in the plugin installation directory. It will need to be the correct version and operating system/architecture.
2. Download the protoc archive and unpack it into the plugin installation directory. It will need to be in a `protoc` directory. Do this if the current version is not supported for your operating system/architecture.
3. Change the `downloadUrl` entry in the plugin manifest to point to a location that the file can be downloaded from.
4. Install the correct version of the protoc compiler as an operating system package. It must then be on the executable path when the plugin runs. For instance, for Alpine Linux this will need to be done as the downloaded versions will not work.

## Supported features

The plugin currently supports proto3 formatted messages and service calls.

It supports the following:
* Scalar fields (Double, Float, Int64, Uint64, Int32, Uint32, Fixed64, Fixed32, Bool, Sfixed32, Sfixed64, Sint32, Sint64).
* Variable length fields (String, Bytes).
* Enum fields.
* Embedded messages.
* Map fields (with a string key).
* Repeated fields.
* Service method calls (requires mocking of gRPC methods as gRPC is not currently supported). 

## Unsupported features

The following features are currently unsupported, but will be supported in a later release:
* oneOf fields.
* default values for fields.
* packed fields.
* required fields.
* gRPC service calls (gRPC mock server).
* Testing/verifying options.

The following features will not be supported by this plugin:
* proto2
* Groups

The following features may be supported in a future release, but are not currently planned to be supported:
* Map fields where the key is not a string

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

For details on the V4 Pact specification, refer to https://github.com/pact-foundation/pact-specification/tree/version-4

For details on the Pact plugin framework, refer to https://github.com/pact-foundation/pact-plugins

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

There is a Pact test that verifies the plugin aqainst the Pact file published to [pact-foundation.pactflow.io](https://pact-foundation.pactflow.io).
Running this test requires a Pactflow API token and the plugin to be built and installed. See the installation instructions above.
The test is run using `cargo test --test pact_verify`.

## Development Roadmap

Pact plugin development board: https://github.com/pact-foundation/pact-plugins/projects/1

## License and Copyright

This plugin is released under the **MIT License** and is copyright © 2021-22 [Pactflow](https://pactflow.io).

The Pactflow logos are copyright © [Pactflow](https://pactflow.io) and may not be used without permission.
