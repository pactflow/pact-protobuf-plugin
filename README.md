<a href="https://pactflow.io"><img src="docs/pactflow-logo-s.png" alt="pactflow logo" height="60px" align="right"></a>

# Pact Protobuf/gRPC Plugin [![Pact-Protobuf-Plugin Build](https://github.com/pactflow/pact-protobuf-plugin/actions/workflows/build.yml/badge.svg)](https://github.com/pactflow/pact-protobuf-plugin/actions/workflows/build.yml)

> Pact plugin for testing messages and gRPC service calls encoded with as [Protocol buffers](https://developers.google.com/protocol-buffers)
> using the [Pact](https://docs.pact.io) contract testing framework.

## About this plugin

This plugin provides support for matching and verifying Protobuf messages and gRPC service calls. It fits into the
[Pact contract testing framework](https://docs.pact.io) and extends Pact testing for [Protocol buffer](https://developers.google.com/protocol-buffers) 
payloads and gRPC. 

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

This plugin provides matching and verification of Protobuf proto3 encoded messages and gRPC service methods to the Pact
contract testing framework. It requires a version of the Pact framework that supports the [V4 Pact specification](https://github.com/pact-foundation/pact-specification/tree/version-4) 
as well as the [Pact plugin framework](https://github.com/pact-foundation/pact-plugins).

Supported Pact framework versions:
- [Pact-JVM v4.4.x](https://github.com/pact-foundation/pact-jvm)
- [Pact-Rust Consumer v0.9.x](https://github.com/pact-foundation/pact-reference/tree/master/rust/pact_consumer)
- [Pact-Rust Verifier v0.9.x](https://github.com/pact-foundation/pact-reference/tree/master/rust/pact_verifier_cli)
- [Pact-Go v2.0.0-beta](https://github.com/pact-foundation/pact-go)

To support compiling Protocol Buffer proto files requires a version of the [Protocol Buffer compiler](https://github.com/protocolbuffers/protobuf).

## Installation

The executable binaries and plugin manifest file for the plugin can be downloaded from the project [releases page](../releases). There will be an executable for each
operating system and architecture. If your particular operating system or architecture is not supported, please send
a request to [support@pactflow.io](support@pactflow.io) with the details.

### Installing the plugin
To install the plugin requires the plugin executable binary as well as the plugin manifest file to be unpacked/copied into
a Pact plugin directory. By default, this will be `.pact/plugins/protobuf-<version>` in the home directory (i.e. 
`$HOME/.pact/plugins/protobuf-0.1.5` for version 0.1.5).

#### Installing the plugin using the pact-plugin-cli

The [pact-plugin-cli](https://github.com/pact-foundation/pact-plugins/tree/main/cli) can be used to install the Protobuf/gRPC
plugin. See the [CLI installation](https://github.com/pact-foundation/pact-plugins/tree/main/cli#installing) on how to install it.

To install the latest version, run

```shell
pact-plugin-cli -y install https://github.com/pactflow/pact-protobuf-plugin/releases/latest
```

#### Manually installing the plugin

Example installation of Linux version 0.1.5 (replace with the actual version you are using): 
1. Create the plugin directory if needed: `mkdir -p ~/.pact/plugins/protobuf-0.1.5`
2. Download the plugin manifest into the directory: `wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-0.1.5/pact-plugin.json -O ~/.pact/plugins/protobuf-0.1.3/pact-plugin.json`
3. Download the plugin executable into the directory: `wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-0.1.5/pact-protobuf-plugin-linux-x86_64.gz  -O ~/.pact/plugins/protobuf-0.1.5/pact-protobuf-plugin.gz`
4. Unpack the plugin executable: `gunzip -N ~/.pact/plugins/protobuf-0.1.5/pact-protobuf-plugin.gz`
5. Make the plugin executable: `chmod +x ~/.pact/plugins/protobuf-0.1.5/pact-protobuf-plugin`

**Note:** The unpacked executable name must match the `entryPoint` value in the manifest file. By default, this is
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
2. Download the protoc archive and unpack it into the plugin installation directory. It will need to be in a `protoc` directory. _Do this if the current version is not supported for your operating system/architecture._
3. Change the `downloadUrl` entry in the plugin manifest to point to a location that the file can be downloaded from.
4. Install the correct version of the protoc compiler as an operating system package. It must then be on the executable path when the plugin runs. For instance, for Alpine Linux this will need to be done as the downloaded versions will not work.

## Logging

_NOTE: Since 0.1.3, the logging was switched to the Rust tracing crate and a log configuration file is no longer supported._ 

The plugin will log to both standard output and two files (log/plugin.log.* and log/plugin.log.json.*) in the plugin 
installation directory. Each file will be rolled per day and be suffixed with the current date. The JSON log file will
be formatted in the [bunyan format](https://github.com/trentm/node-bunyan).The log level will be set by the `LOG_LEVEL`
environment variable that is passed into the plugin process (this should be set by the framework calling it).

## Configuration

The Protobuf plugin supports the following configuration options, which can be set in the plugin manifest file under
`pluginConfig`:

#### `protocVersion` [string]

The Protobuf compiler version to download if required.

#### `downloadUrl` [string]

The URL to download the Protobuf compiler from. By default, this will be the Protocol Buffers GitHub release page.

#### `hostToBindTo` [string]

Host to bind to. Default is the IP4 loopback adapter `127.0.0.1`, to use the IP6 loopback set it to `::1`. 

#### `additionalIncludes` [string or list\<string\>]

Additional directories to include to add to the Protocol buffers compiler to search for proto files. Each value will be
added verbatim to the protoc command line using `-I`. **THESE ARE DIRECTORIES NOT FILES!**

### Specifying configuration values in the tests

*Version 0.2.4+*

Configuration values can also be passed in from the test. They need to be passed in via the [test configuration data](#the-protobuf-test-configuration)
under the `pact:protobuf-config` key. For example, to add additional proto file include directories in the test:

```java
  "pact:proto", filePath("../proto/test_enum.proto"),
  "pact:content-type", "application/grpc",
  "pact:proto-service", "Test/GetFeature",
  "pact:protobuf-config", Map.of(
    "additionalIncludes", List.of(filePath("../proto2"))
  )
```

## Supported features

The plugin currently supports proto3 formatted messages and service calls.

It supports the following:
* Scalar fields (Double, Float, Int64, Uint64, Int32, Uint32, Fixed64, Fixed32, Bool, Sfixed32, Sfixed64, Sint32, Sint64).
* Variable length fields (String, Bytes).
* Enum fields.
* Embedded messages.
* Map fields (with a string key).
* Repeated fields.
* Packed repeated fields.
* oneOf fields.
* gRPC Service method calls. 
* Testing/verifying gRPC service call metadata.
* Verifying gRPC error responses.  

## Unsupported features

The following features are currently unsupported, but may be supported in a later release:
* Map fields with scalar keys.
* Map fields with enum keys.
* default values for fields.
* required fields (note that this is deprecated in Proto 3).
* Testing/verifying Protobuf options.

The following features will **not** be supported by this plugin:
* proto2
* Groups

The following features may be supported in a future release, but are not currently planned to be supported:
* Map fields where the key is not a string or scalar value.
* gRPC streaming (either oneway or bidirectional).

## Using the plugin

This plugin will register itself with the Pact framework for the `application/protobuf` and `application/gRPC` content types.

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

### Testing a gRPC service method interaction

With a service method call, the consumer creates an input message, then invokes a service method and gets an output message
as the response. The most common service call is via the gRPC framework.

#### Testing a gRPC service method interaction with a gRPC server

This plugin supports testing service method calls via gRPC on both the consumer and provider side.

##### Service method consumer

The service method consumer is tested by configuring a test that starts a gRPC mock server based on the proto file for
the service. Each test first configures a Pact from the proto file. The Pact framework (via this plugin)
will then create a gRPC mock server for the test. The gRPC consumer can then be pointed at the mock server during the
test and send the input message and then verify the output message that is received back.

For an example:
* [JVM example gRPC consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/examples/gRPC/area_calculator/consumer-jvm/src/test/java/io/pact/example/grpc/consumer/PactConsumerTest.java)
* [Rust example gRPC consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/examples/gRPC/area_calculator/consumer-rust/src/lib.rs)

##### Service method provider

The Pact framework (using this plugin) can test gRPC service method calls to a running gRPC server. The server can be
tested by either using a unit test, or by using the Rust Verifier CLI. It will need the Pact file created from the
consumer test with the compiled protobuf descriptors in it (these will have been added by this plugin during the consumer test).

For an example Java unit test: See the [example gRPC verification test](https://github.com/pact-foundation/pact-plugins/blob/main/examples/gRPC/area_calculator/provider-jvm/server/src/test/java/io/pact/example/grpc/provider/PactVerificationTest.java).

By starting the gRPC server, we can then also use the [Pact Verifier](https://github.com/pact-foundation/pact-reference/tree/master/rust/pact_verifier_cli) to check it.

###### For example (using the [example gRPC project](https://github.com/pact-foundation/pact-plugins/blob/main/examples/gRPC/area_calculator/provider-jvm)):

_Running the gRPC server:_

```console
gRPC/area_calculator/provider-jvm: 
❯ ./gradlew run

> Task :server:run
14:56:17,785 |-INFO in ch.qos.logback.classic.LoggerContext[default] - Could NOT find resource [logback-test.xml]
14:56:17,786 |-INFO in ch.qos.logback.classic.LoggerContext[default] - Could NOT find resource [logback.groovy]
14:56:17,787 |-INFO in ch.qos.logback.classic.LoggerContext[default] - Found resource [logback.xml] at [file:/home/ronald/Development/Projects/Pact/pact-plugins/examples/gRPC/area_calculator/provider-jvm/server/build/resources/main/logback.xml]
14:56:17,862 |-INFO in ch.qos.logback.classic.joran.action.ConfigurationAction - debug attribute not set
14:56:17,863 |-INFO in ch.qos.logback.core.joran.action.AppenderAction - About to instantiate appender of type [ch.qos.logback.core.ConsoleAppender]
14:56:17,869 |-INFO in ch.qos.logback.core.joran.action.AppenderAction - Naming appender as [STDOUT]
14:56:17,874 |-INFO in ch.qos.logback.core.joran.action.NestedComplexPropertyIA - Assuming default type [ch.qos.logback.classic.encoder.PatternLayoutEncoder] for [encoder] property
14:56:17,928 |-INFO in ch.qos.logback.classic.joran.action.RootLoggerAction - Setting level of ROOT logger to ERROR
14:56:17,928 |-INFO in ch.qos.logback.core.joran.action.AppenderRefAction - Attaching appender named [STDOUT] to Logger[ROOT]
14:56:17,930 |-ERROR in ch.qos.logback.core.joran.spi.Interpreter@13:36 - no applicable action for [io.grpc.netty], current ElementPath  is [[configuration][io.grpc.netty]]
14:56:17,930 |-INFO in ch.qos.logback.classic.joran.action.ConfigurationAction - End of configuration.
14:56:17,932 |-INFO in ch.qos.logback.classic.joran.JoranConfigurator@2b662a77 - Registering current configuration as safe fallback point

Started calculator service on 37621
<===========--> 87% EXECUTING [18s]
> :server:run
```

We can see that the gRPC server was started on a random port (37621 above). So we can then provide the Pact file and
port number to the verifier.

```console
gRPC/area_calculator/provider-jvm: 
❯ pact_verifier_cli -f ../consumer-jvm/build/pacts/protobuf-consumer-area-calculator-provider.json -p 37621

Verifying a pact between protobuf-consumer and area-calculator-provider

  calculate rectangle area request

  Test Name: io.pact.example.grpc.consumer.PactConsumerTest.calculateRectangleArea(MockServer, SynchronousMessages)

  Given a Calculator/calculate request
      with an input .area_calculator.ShapeMessage message
      will return an output .area_calculator.AreaResponse message [OK]
```

#### Testing a gRPC service method interaction without a gRPC server

If you can mock out the gRPC channel or stub, it is fairly easy to test the service method call without requiring a
gRPC server.

##### Service method consumer

To test the service message consumer, we write a Pact test that defines the expected input (or request) message and the 
expected output (or response message). The Pact test framework will generate an example input and output message.

To execute the test, we need to intercept the service method call and verify that the message the consumer generated was
correct, then we return the output message and verify that the consumer processed it correctly. This can be achieved using
a test mocking library.

For an example:
* [JVM example service consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/drivers/jvm/core/src/test/groovy/io/pact/plugins/jvm/core/DriverPactTest.groovy#L116)
* [Rust example service consumer test](https://github.com/pact-foundation/pact-plugins/blob/main/drivers/rust/driver/tests/pact.rs#L43)

##### Service method provider

The Protocol Buffer service providers normally extend an interface generated by the protoc compiler. To test them, we
need a mechanism to get the Pact verifier to pass in the input message from the Pact file and then get the output message
from the service and compare that to the output message from the Pact file.

We can use the same mechanism as for message pact (see [Non-HTTP testing (Message Pact)](https://docs.pact.io/getting_started/how_pact_works/#non-http-testing-message-pact)),
were we create an HTTP proxy server to receive the input message from the verifier and invoke the service method implementation
to get the output message.

There are two main ways to run the verification:

1. Execute the Pact verifier, providing the source of the Pact file, and configure it to use the HTTP mock server.
2. Write a test in the provider's code base. For an example of doing this in Rust, see [a test that verifies this plugin](tests/pact_verify.rs).

#### Verifying gRPC error responses (0.3.1+)

You can use this plugin to test negative cases where an error response is expected to be returned (for an example see
[gRPC status](https://github.com/pact-foundation/pact-plugins/tree/main/examples/gRPC/grpc_status)). This works by
checking if there is an expected `grpc-status` attribute set when an error response is returned (and also can check
for a `grpc-message` attribute).

To use it, don't configure a response message, but add the expected status (and message if required) to the response
metadata. I.e, in the consumer test:

```java
.with(Map.of(
    "pact:proto", filePath("../proto/grpc_status.proto"),
    "pact:content-type", "application/grpc",
    "pact:proto-service", "Calculator/calculate",

    "request", Map.of(
      "parallelogram", Map.of(
        "base_length", "matching(number, 3)",
        "height", "matching(number, 4)"
      )
    ),

    // We are expecting an error response for this message
    "responseMetadata", Map.of(
      "grpc-status", "UNIMPLEMENTED",
      "grpc-message", "matching(type, 'we do not currently support parallelograms')"
    )
))
```

then when the interaction is verified you will see (assuming the provider returns the correct response):

```console
Verifying a pact between grpc-consumer-rust and grpc-provider

  invalid request (0s loading, 54ms verification)

  Given a Calculator/calculate request
      with an input .area_calculator.ShapeMessage message
      will return an error response Operation is not implemented or not supported [OK]
        with metadata
          key 'grpc-message' with value 'we do not currently support parallelograms' [OK]
          key 'grpc-status' with value 'UNIMPLEMENTED' [OK]
```

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

## Running within docker containers

The plugin will try to use an IP6 address when opening the port for the gRPC server. Docker will only support IP6
addresses with extra configuration applied and this will not be available by default. To use an IP4 address instead,
you can either add the host parameter as a command line parameter, or add `hostToBindTo` value to the plugin 
configuration in the manifest file.

I.e., updated manifest to use 127.0.0.1 as the host to bind to

```json
{
  "manifestVersion": 1,
  "pluginInterfaceVersion": 1,
  "name": "protobuf",
  "version": "0.1.8",
  "executableType": "exec",
  "entryPoint": "pact-protobuf-plugin",
  "pluginConfig": {
    "protocVersion": "3.19.1",
    "downloadUrl": "https://github.com/protocolbuffers/protobuf/releases/download",
    "hostToBindTo": "127.0.0.1"
  }
}
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

To build the plugin, you need a working Rust environment (version 1.58+). Refer to the [Rust Guide](https://www.rust-lang.org/learn/get-started).

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
