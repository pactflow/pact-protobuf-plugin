# Pact Protobuf Plugin

Pact plugin for Protocol Buffers and gRPC. Implements version 1 of the [Pact plugin interface](https://github.com/pact-foundation/pact-plugins/blob/main/docs/content-matcher-design.md).

## Build & Test

Rust project using cargo. Edition 2024.

```bash
# Build
cargo build            # debug
cargo build --release  # release (for installing as plugin)

# Unit tests
cargo test --lib

# Integration tests
cargo test --test each_value_tests
cargo test --test basic_values_test
cargo test --test enum_tests

# All tests except PactFlow verify (needs PACTFLOW_TOKEN)
cargo test --test '*' -- --skip verify_plugin
```

**Default branch is `main`.** Conventional Commits required (`feat:`, `fix:`, `chore:`, etc.). PRs are NOT squash-merged — submit a single, clean commit per PR.

### Local pact_models patches

**WARNING:** `Cargo.toml` may have an active `[patch.crates-io]` section pointing `pact_models` to `../pact-reference/rust/pact_models`. If so, `cargo build` will FAIL unless you have a sibling checkout of pact-reference at that path. For normal plugin-only work, comment out the `[patch.crates-io]` section to use the published crate.

### Plugin Process Model

The pact framework loads the plugin as a **subprocess**. The plugin starts a gRPC server and prints `{"port": N, "serverKey": "..."}` to stdout. Consumer tests start the **installed** plugin from `~/.pact/plugins/protobuf-<version>/`, NOT the local build.

To test changes end-to-end:
1. `cargo build` (debug is fine for testing)
2. Copy the binary and manifest to the plugin directory:
   ```bash
   mkdir -p ~/.pact/plugins/pact-protobuf-plugin
   cp pact-plugin.json target/debug/pact-protobuf-plugin ~/.pact/plugins/pact-protobuf-plugin/
   ```
   (CI does the same. For release builds, use `target/release/` and the versioned dir `protobuf-<version>/`.)
3. Run consumer/provider tests

### Integrated Tests

The `integrated_tests/` directory contains separate cargo projects (each with their own Cargo.toml). These are NOT part of the workspace. Each subdirectory tests a specific scenario (e.g., `repeated_enums`, `matching_maps`, `imported_message`, `new_fields`).

Some have consumer + provider subdirectories — run consumer FIRST (generates pact files), then provider (verifies against them). These tests use the installed plugin binary, so you must copy your build there first.

```bash
cd integrated_tests/repeated_field_contains/consumer
cargo test

cd ../provider
cargo test
```

CI runs a specific list of integrated tests (see `.github/workflows/build.yml` `integrated-tests` job). New test directories must be added to that list explicitly.

## Architecture

### Module Map

| Module | File | Purpose |
|--------|------|---------|
| `server` | `src/server.rs` | PactPlugin gRPC service implementation. Entry point for all plugin protocol calls (configure_interaction, compare_contents, verify_interaction, etc.) |
| `protobuf` | `src/protobuf/mod.rs` | Converts JSON test config into protobuf field values + matching rules. The largest and most complex module. |
| `matching` | `src/matching.rs` | Protobuf-specific matching logic. Delegates to `pact_matching` crate for list/map comparison. |
| `metadata` | `src/metadata.rs` | gRPC metadata header processing and matching. |
| `message_builder` | `src/message_builder.rs` | Constructs protobuf wire-format messages from MessageFieldValue trees. |
| `message_decoder` | `src/message_decoder/` | Decodes protobuf wire-format messages back into ProtobufField trees for verification. |
| `verification` | `src/verification.rs` | Orchestrates provider verification: makes gRPC call, decodes response, runs matching. |
| `dynamic_message` | `src/dynamic_message.rs` | gRPC codec using Pact interactions (encode/decode for tonic). |
| `mock_server` | `src/mock_server.rs` | gRPC mock server for consumer tests. |
| `mock_service` | `src/mock_service.rs` | Service implementation backed by Pact interactions for the mock server. |
| `protoc` | `src/protoc.rs` | Proto compiler wrapper — uses the embedded `protox` crate to parse .proto files into a FileDescriptorSet. |
| `tcp` | `src/tcp.rs` | `TcpIncoming` — bridges `TcpListener` to tonic's `Stream` for the gRPC server. |
| `utils` | `src/utils.rs` | Shared helpers: descriptor lookup, name manipulation, route building. |

### Main Entry Point

`src/main.rs` — Starts the gRPC server with auth interceptor, compression, tracing. Auto-shuts down after 10 minutes of inactivity (configurable via `--timeout`).

### Data Flow

**Consumer side (configure_interaction):**
1. Test provides JSON config with `pact:proto`, `pact:proto-service`, request/response bodies
2. `server.rs` → `protobuf::process_proto()` parses the .proto file via the embedded `protox` compiler
3. For each field in the config, `protobuf/mod.rs` builds:
   - A protobuf wire-format message (example value)
   - Matching rules (how to compare during verification)
   - Generators (for dynamic values like UUIDs)
4. Results written to pact file

**Provider side (verify_interaction):**
1. `verification.rs` makes actual gRPC call to provider
2. Response decoded by `message_decoder/`
3. `matching.rs` compares decoded fields against expected fields using matching rules

## Expression Parsing — Call Sites

The function `parse_matcher_def` from `pact_models::matchingrules::expressions` is called 6 times. Each serves a different code path. This is critical knowledge for anyone adding matchers.

### 1. `build_embedded_message_field_value` in `src/protobuf/mod.rs`
Repeated fields with `pact:match` Object config. Handles EachValue, ArrayContains with references. THE entry point for complex repeated field matchers (object form).

### 2. `build_single_embedded_field_value` in `src/protobuf/mod.rs`
Singular embedded message fields with `pact:match`. Generic rule passthrough for non-repeated message fields.

### 3. `build_proto_value` in `src/protobuf/mod.rs`
`google.protobuf.Struct` field values. Generic rule passthrough for well-known Struct types.

### 4. `build_map_field` in `src/protobuf/mod.rs`
Map fields with `pact:match`. Generic rule passthrough. Maps already have "contains key" semantics by default.

### 5. `construct_value_from_string` in `src/protobuf/mod.rs`
Scalar/primitive fields from string expressions. THE entry point for simple repeated primitive matchers like `"field": "eachValue(matching(type, 'X'))"`. Has special path correction for values matchers (`is_values_matcher` + wildcard path → parent path).

### 6. `process_metadata` in `src/metadata.rs`
gRPC metadata headers. String key-values only.

To find these, grep for `parse_matcher_def` — there are exactly 6 call sites (as of this writing).

## Repeated Field Matching Flow

### String form (primitives)
```
Consumer writes: "networking": "eachValue(matching(type, 'PUBLIC'))"
→ Value is String → build_field_value dispatches to construct_value_from_string with path $.networking.*
→ Expression parsed → EachValue rule added to $.networking (path corrected from wildcard)
→ Pact file written with matching rule on $.networking
→ Provider verification: compare_repeated_field checks matcher_is_defined($.networking)
  → delegates to compare_lists_with_matchingrule from pact_matching crate
```

### Object form (embedded messages)
```
Consumer writes: "networking": { "pact:match": "eachValue(...)", "ref": {...} }
→ Value is Object → build_embedded_message_field_value → call site 1
```

### Dispatch logic
`build_field_value` is the main dispatcher. For String values on repeated fields, it calls `construct_value_from_string` with path `$.field.*`. For Object values on repeated fields, the caller (`construct_message_field`) routes to `build_embedded_message_field_value` before `build_field_value` is reached.

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `pact_models` | Matching rule types, expression parser (`parse_matcher_def`), DocPath |
| `pact_matching` | Core comparison logic (`compare_lists_with_matchingrule` handles EachValue, MinType, MaxType, ArrayContains) |
| `pact_consumer` | Consumer DSL (dev-dependency, used in integration tests) |
| `pact_verifier` | Provider verification framework (used in verification module + integration tests) |
| `prost` / `prost-types` | Protobuf encoding/decoding, descriptor types |
| `tonic` | gRPC framework (plugin server + mock server + verification client) |

## CI (`.github/workflows/build.yml`)

Four jobs on PR to `main`:
- **build** — clippy (Linux only) + unit tests + integration tests (skipping pact_verify). Runs on ubuntu, windows, macos.
- **musl-build** — static Linux build via Docker (`pactfoundation/rust-musl-build`)
- **pact-verify** — verifies the plugin itself against PactFlow broker (needs `PACTFLOW_TOKEN`)
- **integrated-tests** — runs each `integrated_tests/` scenario sequentially. New dirs must be added to the explicit list.

No Makefile. All commands are raw cargo.

## Known Issues

- **`atLeast(N)` alone on repeated fields**: Without `eachValue`, MinType goes on `$.field.*` but nothing on `$.field`, so `compare_repeated_field` falls to exact matching. Wrap with `eachValue` to work correctly.
- **Plugin version coupling**: Consumer tests in `integrated_tests/` reference specific plugin versions. The installed plugin version must match what tests expect.
- **Patch dependency**: The `[patch.crates-io]` for `pact_models` requires `../pact-reference` to be checked out. Comment it out if working without local model changes.

## Testing Patterns

- **Unit tests**: `#[cfg(test)] mod tests` blocks in each source file. `src/protobuf/tests/` has a separate test module with `build_field_value_tests.rs`.
- **Integration tests**: `tests/` directory — `each_value_tests.rs`, `basic_values_test.rs`, `enum_tests.rs`, `pact_verify.rs`, `mock_server_tests.rs`, `from_provider_state_generator_tests.rs`. These create pacts and verify matching rules within the same cargo project.
- **End-to-end tests**: `integrated_tests/` — separate cargo projects with consumer + provider subdirs. Consumer generates pact files, provider verifies against them. Uses the installed plugin binary.
