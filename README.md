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
    - [Testing a gRPC service interaction](#testing-a-grpc-service-interaction)
- [Support](#support)
- [Contributing to the plugin](#contributing)
- [Development Roadmap](#development-roadmap)

## Requirements to use it

This plugin provides matching and verification of Protobuf proto3 encoded messages to the Pact contract testing framework. It requires a version
of the Pact framework that supports the [V4 Pact specification](https://github.com/pact-foundation/pact-specification/tree/version-4) 
as well as the [Pact plugin framework](https://github.com/pact-foundation/pact-plugins).

Supported Pact versions:
- [Pact-JVM v4.3.x](https://github.com/pact-foundation/pact-jvm)
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
* RPC Service method calls (requires mocking of gRPC methods calls as gRPC is not currently supported). 

## Unsupported features

The following features are currently unsupported, but will be supported in a later release:
* oneOf fields.
* Map fields with scalar keys.
* Map fields with enum keys.
* default values for fields.
* packed fields.
* required fields.
* gRPC service calls (gRPC mock server).
* Testing/verifying options.

The following features will **not** be supported by this plugin:
* proto2
* Groups

The following features may be supported in a future release, but are not currently planned to be supported:
* Map fields where the key is not a string or scalar value.
* gRPC streaming (either oneway or bidirectional).

## Using the plugin

This plugin will register itself with the Pact framework for the `application/protobuf` content type.

Using this plugin, you can write Pact tests that verify either a single Protobuf message (i.e. a message provider sends
a single, or one-shot, message to a consumer), or you can verify a service method call where there is an input message 
and an output message.

Single message tests are supported by using the V4 asynchronous message Pact format, and the service method calls use the
V4 synchronous message Pact format.

### Testing an interaction with a single Protobuf message

For an overview how asynchronous messages work with Pact, see [Non-HTTP testing (Message Pact)](https://docs.pact.io/getting_started/how_pact_works/#non-http-testing-message-pact).

In this scenario, a message provider writes a Protocol Buffer message to some one-way transport mechanism, like a message queue, and a consumer
then reads it. With this style of testing, the transport mechanism is abstracted away.

#### Protocol Buffer message consumer

The message consumer test is written using the Pact Message test DSL. The test DSL defines the expected message format,
and then the consumer is tested with an example message generated by the test framework.

For an example of a message consumer test:
* [Java example consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/examples/protobuf/protobuf-consumer-jvm/src/test/java/io/pact/example/protobuf/provider/PactConsumerTest.java)
* [Rust example consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/examples/protobuf/protobuf-consumer-rust/src/lib.rs)

#### Verifying the message provider

The message provider is verified by getting it to generate a message, and then this is verified against the Pact file
from the consumer. There are two main ways of verifying the provider:

1. Write a test in the provider code base that can call the provider to generate the message. 
2. Use an HTTP proxy server that can call the provider and return the generated message, and then use a Pact framework verifier to verify it.

For an example of the latter form, see [Simple Example Protobuf provider](https://github.com/pact-foundation/pact-plugins/tree/main/examples/protobuf/protobuf-provider).

### Testing an RPC service method interaction

**NOTE: gRPC service calls are not currently supported directly, but will be supported in a future version.**

With a service method call, the consumer creates an input message, then invokes a service method and gets an output message
as the response. The most common service call is via the gRPC RPC framework.

#### Protocol Buffer service message consumer

To test the service message consumer, we write a Pact test that defines the expected input (or request) message and the 
expected output (or response message). The Pact test framework will generate an example input and output message.

To execute the test, we need to intercept the service method call and verify that the message the consumer generated was
correct, then we return the output message and verify that the consumer processed it correctly. This can be achieved using
a test mocking library.

For an example:
* [JVM example service consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/drivers/jvm/core/src/test/groovy/io/pact/plugins/jvm/core/DriverPactTest.groovy#L116)
* [Rust example service consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/drivers/rust/driver/tests/pact.rs#L43)

#### Protocol Buffer service message provider

The Protocol Buffer service providers normally extend an interface generated by the protoc compiler. To test them, we
need a mechanism to get the Pact verifier to pass in the input message from the Pact file and then get the output message
from the service and compare that to the output message from the Pact file.

We can use the same mechanism as for message pact (see [Non-HTTP testing (Message Pact)](https://docs.pact.io/getting_started/how_pact_works/#non-http-testing-message-pact)),
were we create an HTTP proxy server to receive the input message from the verifier and invoke the service method implementation
to get the output message.

There are two main ways to run the verification:

1. Execute the Pact verifier, providing the source of the Pact file, and configure it to use the HTTP mock server.
2. Write a test in the provider's code base. For an example of doing this in Rust, see [a test that verifies this plugin](tests/pact_verify.rs).

### The Protobuf test configuration

The consumer tests need to get the plugin loaded and configure the expected messages to use in the test. This is done
using the `usingPlugin` (or `using_plugin`, depending on the language implementation) followed by the content for the test
in some type of map form.

For each field of the message that we want in the contract, we define an entry with the field name as the key and
a matching definition as the value. For documentation on the matching definition format, see [Matching Rule definition expressions](https://github.com/pact-foundation/pact-plugins/blob/main/docs/matching-rule-definition-expressions.md).

For example, for a JVM test (taken from [Protocol Buffer Java examples](https://developers.google.com/protocol-buffers/docs/javatutorial)) we would use the PactBuilder class:

```protobuf
// this example taken from https://developers.google.com/protocol-buffers/docs/javatutorial#defining-your-protocol-format
message Person {
  string name = 1;
  int32 id = 2;
  string email = 3;

  enum PhoneType {
    MOBILE = 0;
    HOME = 1;
    WORK = 2;
  }

  message PhoneNumber {
    string number = 1;
    PhoneType type = 2 [default = HOME];
  }

  repeated PhoneNumber phones = 4;
}
```

```java
builder
  // Tell the Pact framework to load the protobuf plugin      
  .usingPlugin("protobuf")
        
  // Define the expected message (description) and the type of interaction. Here is is an asynchronous message.
  .expectsToReceive("Person Message", "core/interaction/message")
        
  // Provide the data for the test
  .with(Map.of(
    // For a single asynchronous message, we just provide the contents for the message. For RPC service calls, there
    // will be a request and response message
    "message.contents", Map.of(
      // set the content type, so the Pact framework will know to send it to the Protobuf plugin
      "pact:content-type", "application/protobuf",
      // pact:proto contains the source proto file, which is required to be able to test the interaction
      "pact:proto", filePath("addressbook.proto"),
      // provide the name of the message type we are going to test (defined in the proto file)
      "pact:message-type", "Person",
      
      // We can then setup the expected fields of the message
      "name", "notEmpty('Fred')", // The name field must not be empty, and we use Fred in our tests
      "id", "matching(regex, '100\\d+', '1000001')", // The id field must match the regular expression, and we use 1000001 in the tests 
      "email", "matching(regex, '\\w+@[a-z0-9\\.]+', 'test@ourtest.com')" // Emails must match a regular expression

      // phones is a repeated field, so we define an example that all values must match against
      "phones", Map.of(
        "number", "matching(regex, '(\\+\\d+)?(\\d+\\-)?\\d+\\-\\d+', '+61-03-1234-5678')" // Phone numbers must match a regular expression
        // We don't include type, as it is an emum and has a default value, so it is optional
        // but we could have done something like matching(equalTo, 'WORK')
      )
    )
  ))
```

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
