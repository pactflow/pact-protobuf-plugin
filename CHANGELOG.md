# 0.6.5 - Maintenance Release

* f0c3abb - fix: finding deeply nested enums (#232) (Eric Muller, Tue Aug 5 17:40:34 2025 -0700)
* 756150f - bump version to 0.6.5 (Ronald Holshausen, Tue Jul 29 11:07:12 2025 +1000)

# 0.6.4 - Bugfix Release

* 6d85ac2 - fix: Consumer expectations were not passed down for repeated message fields #231 (Ronald Holshausen, Tue Jul 29 10:49:45 2025 +1000)
* b13f0d6 - bump version to 0.6.4 (Ronald Holshausen, Mon Jul 28 09:28:55 2025 +1000)

# 0.6.3 - Bugfix Release

* 940ee6b - fix: Only need to check the start of the content type string #32 (Ronald Holshausen, Wed Jul 23 15:43:24 2025 +1000)
* b76316b - feat: Update readme with a note about byte arrays with non-utf8 data #229 (Ronald Holshausen, Wed Jul 23 11:48:26 2025 +1000)
* 6f3b414 - feat: Support byte arrays with non-utf8 data #229 (Ronald Holshausen, Wed Jul 23 11:09:57 2025 +1000)
* 11ee30b - bump version to 0.6.3 (Ronald Holshausen, Thu Jul 17 14:46:02 2025 +1000)

# 0.6.2 - Maintenance Release

* 3b2a473 - chore: Small optimisation, delay locking the state mutex as late as possible (Ronald Holshausen, Wed Jul 9 10:15:26 2025 +1000)
* a9bdb80 - test: Test atLeast and atMost first in expression (#226) (Tien Vo Xuan, Wed Jul 9 06:55:59 2025 +0700)
* 8102970 - feat: Support fromProviderState generator in regex matching expression (#225) (Tien Vo Xuan, Wed Jul 9 06:55:34 2025 +0700)
* d195949 - bump version to 0.6.2 (Ronald Holshausen, Tue Jul 8 15:57:22 2025 +1000)

# 0.6.1 - Bugfix Release

* 54060e8 - chore: Update other builds to latest Rust stable (Ronald Holshausen, Tue Jul 8 14:00:21 2025 +1000)
* 0fe97db - chore: Update musl build to Rust 1.88 (Ronald Holshausen, Tue Jul 8 13:54:11 2025 +1000)
* f348a47 - fix(mock-server): Need to handle the case where there are multiple messages per route #228 (Ronald Holshausen, Tue Jul 8 11:19:44 2025 +1000)
* 91d0f75 - bump version to 0.6.1 (Ronald Holshausen, Wed Jun 25 10:24:35 2025 +1000)

# 0.6.0 - Fix for always requiring repeated enum field values

* 3bb48cf - chore: DRY up some of the error responses (Ronald Holshausen, Wed Jun 25 09:59:26 2025 +1000)
* 2c93c0f - fix: Use the stored consumer expectations to determine if a repeated field is missing #197 (Ronald Holshausen, Wed Jun 25 09:43:15 2025 +1000)
* 8de0573 - fix: Store the expectations from the consumer test in the Pact file #197 (Ronald Holshausen, Tue Jun 24 11:36:49 2025 +1000)
* 3c3d946 - chore: Bump minor version (Ronald Holshausen, Tue Jun 24 11:32:44 2025 +1000)
* 8c05cbc - chore: Update all dependencies and upgrade Tonic to 0.13.1 (Ronald Holshausen, Tue Jun 24 10:09:22 2025 +1000)
* 6cca3f4 - chore: Upgrade minor dependencies + pact depenencies to latest (Ronald Holshausen, Mon Jun 23 15:50:41 2025 +1000)
* 18632b0 - chore: Update prohect to Rust 2024 edition (Ronald Holshausen, Mon Jun 23 14:01:26 2025 +1000)
* c50ee6b - Revert "Add .github/renovate.json" (Ronald Holshausen, Mon Jun 23 09:36:11 2025 +1000)
* 95eff94 - fix(deps): update rust crate reqwest to v0.12.20 (#224) (pactflow-renovate-bot[bot], Fri Jun 20 19:44:00 2025 +0000)
* 2fcda04 - fix(deps): update rust crate clap to v4.5.40 (#221) (pactflow-renovate-bot[bot], Thu Jun 19 18:54:12 2025 +0000)
* d94d1df - fix(deps): update rust crate os_info to v3.12.0 (#213) (pactflow-renovate-bot[bot], Wed Jun 18 08:18:14 2025 +0000)
* d963b94 - fix(deps): update tokio-tracing monorepo (pactflow-renovate-bot[bot], Mon Jun 16 10:16:21 2025 +0000)
* 3cbc20f - fix(deps): update rust crate reqwest to v0.12.19 (#198) (pactflow-renovate-bot[bot], Thu Jun 12 13:02:50 2025 +0000)
* f23f8b4 - fix(deps): update rust crate hyper-util to v0.1.14 (pactflow-renovate-bot[bot], Sat Jun 7 16:51:39 2025 +0000)
* 2f1c631 - fix(deps): update rust crate tower-http to v0.6.6 (pactflow-renovate-bot[bot], Sat Jun 7 16:51:49 2025 +0000)
* da11fb7 - fix(deps): update rust crate reqwest to v0.12.18 (#196) (pactflow-renovate-bot[bot], Sat Jun 7 16:51:07 2025 +0000)
* c4743d6 - fix(deps): update rust crate reqwest to v0.12.17 (#195) (pactflow-renovate-bot[bot], Sat Jun 7 13:52:07 2025 +0000)
* 5186456 - fix(deps): update rust crate reqwest to v0.12.16 (#194) (pactflow-renovate-bot[bot], Fri Jun 6 23:44:22 2025 +0000)
* 53df1f5 - fix(deps): update rust crate clap to v4.5.39 (#187) (pactflow-renovate-bot[bot], Fri Jun 6 18:17:37 2025 +0000)
* 1385d5d - fix: Update GHA runners to latest windows image OP-33207 (Daniel García-Magán Correas, Wed Jun 4 16:43:06 2025 +0100)
* 1eded4e - fix(deps): update rust crate hyper-util to v0.1.13 (pactflow-renovate-bot[bot], Mon Jun 2 05:05:27 2025 +0000)
* 161a14e - fix(deps): update rust crate tokio to v1.45.1 (pactflow-renovate-bot[bot], Mon Jun 2 05:05:44 2025 +0000)
* c625f40 - fix(deps): update rust crate tower-http to v0.6.5 (pactflow-renovate-bot[bot], Mon Jun 2 05:05:55 2025 +0000)
* 81bfb8a - fix(deps): update rust crate clap to v4.5.38 (pactflow-renovate-bot[bot], Mon Jun 2 05:38:07 2025 +0000)
* 9b8f89b - fix(deps): update rust crate chrono to v0.4.41 (pactflow-renovate-bot[bot], Mon Jun 2 04:29:07 2025 +0000)
* 6108e22 - fix(deps): update rust crate clap to v4.5.38 (pactflow-renovate-bot[bot], Mon Jun 2 04:29:13 2025 +0000)
* 257694b - fix(deps): update rust crate hyper-util to v0.1.12 (pactflow-renovate-bot[bot], Mon Jun 2 04:29:23 2025 +0000)
* 02d5233 - fix(deps): update rust crate tower-http to v0.6.4 (pactflow-renovate-bot[bot], Mon Jun 2 04:29:32 2025 +0000)
* 8ac8f9c - chore(deps): update rust crate built to 0.8.0 (pactflow-renovate-bot[bot], Mon Jun 2 04:29:45 2025 +0000)
* 7f51026 - fix(deps): update rust crate os_info to v3.11.0 (pactflow-renovate-bot[bot], Mon Jun 2 04:30:02 2025 +0000)
* 0ac11fb - fix(deps): update rust crate tempfile to v3.20.0 (pactflow-renovate-bot[bot], Mon Jun 2 04:30:36 2025 +0000)
* 250e9e3 - fix(deps): update rust crate tokio to v1.45.0 (pactflow-renovate-bot[bot], Mon Jun 2 04:30:47 2025 +0000)
* 07254c8 - fix(deps): update rust crate uuid to v1.17.0 (pactflow-renovate-bot[bot], Mon Jun 2 02:44:57 2025 +0000)
* b97dac9 - chore: upgrade ubuntu runner (JP-Ellis, Mon Jun 2 13:44:34 2025 +1000)
* ffd8ffb - fix(deps): update rust crate anyhow to v1.0.97 (JP-Ellis, Tue Apr 8 00:19:24 2025 +0000)
* c8ad53c - fix(deps): update rust crate anyhow to v1.0.98 (pactflow-renovate-bot[bot], Thu Apr 24 00:53:26 2025 +0000)
* d90888c - Update README.md (ZahraKhanRed, Wed Apr 23 13:13:48 2025 +0100)
* 4a98338 - fix(deps): update rust crate clap to v4.5.37 (pactflow-renovate-bot[bot], Tue Apr 22 05:53:37 2025 +0000)
* c83cd73 - fix(deps): update rust crate clap to v4.5.36 (pactflow-renovate-bot[bot], Mon Apr 21 20:25:48 2025 +0000)
* 98d81e8 - fix(deps): update rust crate hyper-util to v0.1.11 (#147) (pactflow-renovate-bot[bot], Thu Apr 10 19:15:21 2025 +0000)
* e29b44f - fix(deps): update rust crate zip to v2.6.1 (pactflow-renovate-bot[bot], Tue Apr 8 07:17:05 2025 +0000)
* 457b655 - fix(deps): update rust crate clap to v4.5.35 (pactflow-renovate-bot[bot], Tue Apr 8 07:16:09 2025 +0000)
* e16af96 - fix(deps): update rust crate tokio [security] (pactflow-renovate-bot[bot], Tue Apr 8 06:16:47 2025 +0000)
* 02b142e - fix(deps): update rust crate tempfile to v3.19.1 (pactflow-renovate-bot[bot], Tue Apr 8 05:18:52 2025 +0000)
* 04418e9 - fix(deps): update rust crate tokio to v1.44.1 (pactflow-renovate-bot[bot], Tue Apr 8 05:19:05 2025 +0000)
* 7abf5dc - fix(deps): update rust crate fake to v4 (pactflow-renovate-bot[bot], Tue Apr 8 02:46:15 2025 +0000)
* 23cb3ef - fix(deps): update rust crate zip to v2.5.0 (pactflow-renovate-bot[bot], Tue Apr 8 05:19:13 2025 +0000)
* 0f7aed9 - fix(deps): update rust crate tracing-bunyan-formatter to v0.3.10 (#90) (pactflow-renovate-bot[bot], Tue Apr 8 05:17:23 2025 +0000)
* b49cbbd - fix(deps): update rust crate thiserror to v2.0.12 (#122) (pactflow-renovate-bot[bot], Tue Apr 8 04:15:38 2025 +0000)
* ee65c62 - fix(deps): update rust crate thiserror to v2 (pactflow-renovate-bot[bot], Tue Apr 8 02:46:21 2025 +0000)
* 7814415 - fix(deps): update rust crate itertools to 0.14.0 (pactflow-renovate-bot[bot], Tue Apr 8 02:45:12 2025 +0000)
* 56d570b - fix(deps): update tokio-tracing monorepo (pactflow-renovate-bot[bot], Tue Apr 8 02:44:54 2025 +0000)
* 45255e6 - fix(deps): update rust crate anyhow to v1.0.97 (pactflow-renovate-bot[bot], Tue Apr 8 02:43:43 2025 +0000)
* 1b704c7 - fix(deps): update rust crate tokio [security] (pactflow-renovate-bot[bot], Tue Apr 8 02:43:01 2025 +0000)
* dc0846f - fix(deps): update rust crate zip to v2.3.0 [security] (pactflow-renovate-bot[bot], Tue Apr 8 02:43:13 2025 +0000)
* e03fac4 - fix(deps): update rust crate http to v1.3.1 (pactflow-renovate-bot[bot], Tue Apr 8 02:45:04 2025 +0000)
* f2ba699 - fix(deps): update rust crate reqwest to v0.12.15 (pactflow-renovate-bot[bot], Tue Apr 8 02:44:19 2025 +0000)
* ebbfe17 - fix(deps): update rust crate hyper to v1.6.0 (pactflow-renovate-bot[bot], Tue Apr 8 00:21:49 2025 +0000)
* 81245a5 - chore(deps): update rust crate built to v0.7.7 (pactflow-renovate-bot[bot], Tue Apr 8 00:19:09 2025 +0000)
* d487028 - fix(deps): update rust crate chrono to v0.4.40 (pactflow-renovate-bot[bot], Tue Apr 8 00:19:41 2025 +0000)
* b363a5f - fix(deps): update rust crate clap to v4.5.34 (pactflow-renovate-bot[bot], Tue Apr 8 00:19:48 2025 +0000)
* 1a7f49e - fix(deps): update tokio-prost monorepo to v0.13.5 (pactflow-renovate-bot[bot], Tue Apr 8 00:20:54 2025 +0000)
* 34b5314 - fix(deps): update rust crate tower to v0.5.2 (pactflow-renovate-bot[bot], Tue Apr 8 00:20:26 2025 +0000)
* 48afca7 - fix(deps): update rust crate bytes to v1.10.1 (pactflow-renovate-bot[bot], Tue Apr 8 00:21:35 2025 +0000)
* db7ab29 - fix(deps): update rust crate async-trait to v0.1.88 (pactflow-renovate-bot[bot], Tue Apr 8 00:19:34 2025 +0000)
* 9a581b0 - fix(deps): update rust crate serde_json to v1.0.140 (pactflow-renovate-bot[bot], Tue Apr 8 00:20:18 2025 +0000)
* 9b9cdb2 - fix(deps): update rust crate tower-http to v0.6.2 (pactflow-renovate-bot[bot], Tue Apr 8 00:20:33 2025 +0000)
* 74b29d3 - chore(deps): update rust crate rstest to 0.25.0 (pactflow-renovate-bot[bot], Tue Apr 8 00:21:28 2025 +0000)
* 99ea9ac - fix(deps): update rust crate os_info to v3.10.0 (pactflow-renovate-bot[bot], Tue Apr 8 00:22:06 2025 +0000)
* 6f0229b - fix(deps): update rust crate uuid to v1.16.0 (pactflow-renovate-bot[bot], Tue Apr 8 00:22:42 2025 +0000)
* e897023 - chore(ci): avoid duplicating ci (JP-Ellis, Tue Apr 8 12:08:50 2025 +1000)
* e35e48b - chore(ci): update rust-musl-build image (JP-Ellis, Tue Apr 8 12:07:40 2025 +1000)
* f0f1e19 - chore(deps): update actions/github-script action to v7 (pactflow-renovate-bot[bot], Mon Apr 7 03:35:39 2025 +0000)
* a3e981d - chore(deps): update actions/checkout action to v4 (pactflow-renovate-bot[bot], Mon Apr 7 03:35:34 2025 +0000)
* 446dd98 - fix(deps): update rust crate regex to v1.11.1 (#104) (pactflow-renovate-bot[bot], Mon Apr 7 07:18:56 2025 +0000)
* a190345 - fix(deps): update rust crate fake to v2.10.0 (#96) (pactflow-renovate-bot[bot], Mon Apr 7 06:16:38 2025 +0000)
* cfb6db3 - chore(deps): update tomhjp/gh-action-jira-search action to v0.2.2 (#94) (pactflow-renovate-bot[bot], Mon Apr 7 05:15:10 2025 +0000)
* 6fd5d0b - fix(deps): update rust crate tonic to v0.12.3 [security] (#78) (pactflow-renovate-bot[bot], Mon Apr 7 04:16:45 2025 +0000)
* 29b0fa6 - Add .github/renovate.json (pactflow-renovate-bot[bot], Fri Feb 14 01:38:07 2025 +0000)
* 4435cac - chore(ci): upgrade macos-12 to macos-13 (JP-Ellis, Fri Dec 6 10:01:41 2024 +1100)
* 132d503 - bump version to 0.5.5 (Ronald Holshausen, Mon Nov 18 10:34:57 2024 +1100)

# 0.5.4 - Bugfix Release

* d6b1626 - fix: Fixed regression introduced by 0.5.3 release (Ronald Holshausen, Mon Nov 18 10:21:10 2024 +1100)
* 7abcf38 - bump version to 0.5.4 (Ronald Holshausen, Fri Nov 15 10:36:17 2024 +1100)

# 0.5.3 - Support for injecting an array of values into a repeated field

* ffa56f2 - chore: Upgrade pact_models to 1.2.5 (Ronald Holshausen, Wed Nov 13 11:17:46 2024 +1100)
* 3d60553 - refactor: Update DynamicMessage to consolidate fields into a single value by field number #73 (Ronald Holshausen, Mon Nov 11 15:55:14 2024 +1100)
* e2e08c9 - refactor: Update value injection to push additional values into the additional_data attribute #73 (Ronald Holshausen, Mon Nov 11 15:09:38 2024 +1100)
* 2abbed2 - refactor: Write out the additional field values when serialising a field #73 (Ronald Holshausen, Mon Nov 11 14:15:12 2024 +1100)
* 47aa5c7 - refactor: Add an attribute to ProtobufField to capture additional values from repeated fields #73 (Ronald Holshausen, Mon Nov 11 11:16:10 2024 +1100)
* c51337d - feat: Add some tests around injecting an array of values into a repeated field #73 (Ronald Holshausen, Wed Nov 6 16:28:56 2024 +1100)
* 25ba86a - feat: Support injecting an array of values into a repeated field #73 (Ronald Holshausen, Wed Nov 6 16:12:09 2024 +1100)
* 1f05ab5 - refactor: Update ProtobufField to also contain the field descriptor for the field #73 (Ronald Holshausen, Fri Nov 1 11:09:58 2024 +1100)
* e5b9de6 - refactor: Update DynamicMessage fetch_field_value and set_field_value to return a result #73 (Ronald Holshausen, Fri Nov 1 09:39:20 2024 +1100)
* cfb54b4 - refactor: Update DynamicMessage to store the Protobuf fields as map keyed by field number #73 (Ronald Holshausen, Thu Oct 31 16:56:56 2024 +1100)
* 8452e25 - refactor: Pass protobuf message descriptor to DynamicMessage::new #73 (Ronald Holshausen, Thu Oct 31 16:28:08 2024 +1100)
* a7b1218 - chore: Update dependencies (Ronald Holshausen, Thu Oct 31 09:42:30 2024 +1100)
* 798b345 - bump version to 0.5.3 (Ronald Holshausen, Mon Aug 26 11:39:37 2024 +1000)

# 0.5.2 - Bugfix Release

* 7475c0f - chore: Fix pact verification request (Ronald Holshausen, Mon Aug 26 11:27:33 2024 +1000)
* 089e864 - chore: Fix pact verification request (Ronald Holshausen, Mon Aug 26 11:19:51 2024 +1000)
* d6865b4 - fix: HTTP Protobuf interactions can have a message defined for both the request and response parts (Ronald Holshausen, Mon Aug 26 10:54:55 2024 +1000)
* 4997452 - bump version to 0.5.2 (Ronald Holshausen, Wed Aug 14 09:47:37 2024 +1000)

# 0.5.1 - Bugfix Release

* 7e6eb62 - chore: fix integrated_tests after upgrading pact consumer crate (Ronald Holshausen, Wed Aug 14 09:36:25 2024 +1000)
* 2eb9d1b - chore: fix integrated_tests after upgrading pact consumer crate (Ronald Holshausen, Wed Aug 14 09:23:35 2024 +1000)
* 090de50 - chore: fix integrated_tests after upgrading pact consumer crate (Ronald Holshausen, Tue Aug 13 16:49:09 2024 +1000)
* e3c32f1 - chore: fix integrated_tests after upgrading pact consumer crate (Ronald Holshausen, Tue Aug 13 16:36:35 2024 +1000)
* f459668 - chore: Upgrade all the Pact dependencies (Ronald Holshausen, Tue Aug 13 16:16:33 2024 +1000)
* c3ec0eb - fix: Handle google.Structs correctly #71 (Ronald Holshausen, Tue Aug 13 10:50:12 2024 +1000)
* 1e5e5bc - bump version to 0.5.1 (Ronald Holshausen, Fri Aug 9 09:37:43 2024 +1000)

# 0.5.0 - Supports provider state injected values and better use of package names when resolving messages

* a9630de - chore: Make test less brittle (Ronald Holshausen, Fri Aug 9 09:16:16 2024 +1000)
* 309f8fc - docs: updated a couple of code comments (Stan Vodetskyi, Thu Aug 8 15:30:16 2024 -0700)
* fcf42b5 - feat: Update readme with example of provider state injected values #69 (Ronald Holshausen, Thu Aug 8 16:15:58 2024 +1000)
* 402ad18 - feat: Enable support for provider state injected values for gRPC metadata #69 (Ronald Holshausen, Thu Aug 8 15:39:29 2024 +1000)
* e001efa - feat: use packages when looking up services and message types. (Stan Vodetskyi, Sat Aug 3 02:34:08 2024 -0700)
* 6064316 - feat: Pass the test context values through to the ProviderStateGenerator #69 (Ronald Holshausen, Wed Aug 7 15:30:42 2024 +1000)
* 7e85d34 - feat: Implement applying generators to mutate the Protobuf messages #69 (Ronald Holshausen, Wed Aug 7 10:40:13 2024 +1000)
* ef237ec - feat: Support for use of provider state generator in consumer tests #69 (Ronald Holshausen, Tue Aug 6 15:04:04 2024 +1000)
* 2b7d5e0 - chore: cleanup some unused code warnings (Ronald Holshausen, Fri Jul 19 09:32:12 2024 +1000)
* c551867 - chore: Remove unused crate (Ronald Holshausen, Thu Jul 18 14:12:52 2024 +1000)
* eda6c17 - chore: Update integration test dependencies (Ronald Holshausen, Thu Jul 18 12:04:11 2024 +1000)
* 8baefcb - chore: Upgrade Tonic to 0.12.0 and Hyper to 1.4.1 (Ronald Holshausen, Thu Jul 18 11:42:11 2024 +1000)
* f7c459a - chore: Update depenendencies (Ronald Holshausen, Thu Jul 18 09:48:12 2024 +1000)
* 2e281ff - Merge pull request #65 from pactflow/docs/binary_compat (Ronald Holshausen, Tue Jun 11 10:47:11 2024 +1000)
* a5ed622 - bump version to 0.4.1 (Ronald Holshausen, Tue Jun 11 10:36:44 2024 +1000)
* 13e9cc0 - chore(docs): update binary compatability (Yousaf Nabi, Mon May 20 13:36:23 2024 +0100)

# 0.4.0 - Fixes message name resolving to correctly use package names

* a1fad28 - chore: Update default protoc compiler to next major version (21.12) (Ronald Holshausen, Tue Jun 11 10:20:14 2024 +1000)
* f219a61 - chore: Bump minor version (Ronald Holshausen, Tue Jun 11 10:12:56 2024 +1000)
* d181915 - remove additional trace logs (Stan, Thu Jun 6 16:28:18 2024 -0700)
* 07652c9 - correct paths in integrated_tests workflows (Stan, Thu Jun 6 16:25:46 2024 -0700)
* a3c41b4 - update mod.rs to also use package (Stan, Thu Jun 6 16:15:55 2024 -0700)
* 8742072 - remove lifetime qualifiers; refactor; remove unnecessary test (Stan, Thu Jun 6 11:42:17 2024 -0700)
* cf9f08b - couple more integrated tests just to be sure (Stan, Wed Jun 5 18:21:23 2024 -0700)
* a8c9f46 - Search descritors with no package when no package was specified. (Stan, Tue Jun 4 23:51:27 2024 -0700)
* ce61d8f - no unwrap (Stan, Tue Jun 4 14:00:16 2024 -0700)
* f4a43d4 - fix tests (Stan, Tue Jun 4 13:17:09 2024 -0700)
* c3278c6 - remove unnecessary changes (Stan, Mon Jun 3 17:20:48 2024 -0700)
* ebe3203 - use crate vs use super (Stan, Mon Jun 3 17:19:16 2024 -0700)
* d13474e - a basic unit test (Stan, Mon Jun 3 17:10:29 2024 -0700)
* c4ac8d7 - remove reference to a deleted method (Stan, Mon Jun 3 16:37:53 2024 -0700)
* 66b97ca - more cases covered in the test (Stan, Mon Jun 3 16:25:56 2024 -0700)
* 2e00c56 - revert formatting and logging changes (Stan, Mon Jun 3 16:19:58 2024 -0700)
* bd1fdc8 - hacky fix cont (Stan, Fri May 31 18:26:34 2024 -0700)
* 18f2e43 - hacky fix (Stan, Fri May 31 16:16:28 2024 -0700)
* 0fc95d2 - conflicting names when package is implicit (Stan, Fri May 31 15:13:35 2024 -0700)
* 0cb87dc - bump version to 0.3.16 (Ronald Holshausen, Fri May 10 10:17:56 2024 +1000)

# 0.3.15 - Bugfix Release

* 63b141f - Merge pull request #57 from YOU54F/feat/slim_bins (Ronald Holshausen, Fri May 10 09:58:25 2024 +1000)
* ccf66da - Merge pull request #56 from YOU54F/feat/musl_linux_win_aarch64 (Ronald Holshausen, Fri May 10 09:57:46 2024 +1000)
* 0f47677 - fix: package namespaces are not respected (Eric Muller, Wed May 8 15:23:02 2024 -0700)
* d8e8b89 - demo bug with conflicting names (Stan, Tue May 7 13:12:18 2024 -0700)
* 652fbd4 - feat: reduce executable size (Yousaf Nabi, Tue Apr 30 17:43:47 2024 +0100)
* 99cdd58 - feat: linux musl static bins / windows aarch64 (Yousaf Nabi, Tue Apr 30 17:43:14 2024 +0100)
* 40abddf - chore(ci): macos-12 (Yousaf Nabi, Tue Apr 30 14:51:11 2024 +0100)
* 905967e - chore: Update dependencies (Ronald Holshausen, Wed Apr 17 10:45:56 2024 +1000)
* 02b236e - chore: Update dependencies (Ronald Holshausen, Wed Apr 17 10:34:31 2024 +1000)
* fb216f1 - chore: Add integrated_tests back into main project (Ronald Holshausen, Tue Apr 16 16:56:02 2024 +1000)
* 454e89d - fix: Upgrade dependencies to fix tests hanging on Windows (Ronald Holshausen, Tue Apr 16 16:50:03 2024 +1000)
* 821047f - Merge branch 'release/0.3.14' (Ronald Holshausen, Fri Apr 12 15:30:06 2024 +1000)
* 713bfe5 - chore: lock jobserver to previous version as 0.1.29 fails to compile (Ronald Holshausen, Fri Apr 12 13:51:50 2024 +1000)
* e4cbc3f - chore: lock jobserver to previous version as 0.1.29 fails to compile (Ronald Holshausen, Fri Apr 12 13:47:00 2024 +1000)
* 6ff8372 - bump version to 0.3.15 (Ronald Holshausen, Fri Apr 12 09:54:37 2024 +1000)

# 0.3.14 - Bugfix Release

* 9ba9b24 - chore: add macos to the release binaries (Ronald Holshausen, Fri Apr 12 09:29:16 2024 +1000)
* d9ce8fb - fix: Take into account package names when looking for message types in the descriptors (Ronald Holshausen, Thu Apr 11 16:11:07 2024 +1000)
* 4587ac7 - fix: Unknown varint fields were incorrectly treated as u64 values with 8 bytes #53 (Ronald Holshausen, Wed Apr 10 15:24:16 2024 +1000)
* d31aac8 - chore: Add example provider with new fields added #53 (Ronald Holshausen, Wed Apr 10 15:18:29 2024 +1000)
* 0b9ac92 - chore: Add example with consumer and provider #53 (Ronald Holshausen, Wed Apr 10 12:07:35 2024 +1000)
* 7414995 - chore: Update dependencies (Ronald Holshausen, Tue Apr 9 14:44:20 2024 +1000)
* fb3faf2 - chore: Update clap to latest (Ronald Holshausen, Tue Apr 9 14:36:32 2024 +1000)
* 56ddeab - chore: Remove locked version of ahash (Ronald Holshausen, Tue Apr 9 13:55:43 2024 +1000)
* f5b6ad9 - chore: Disable test that hangs in CI Windows agents (Ronald Holshausen, Tue Apr 9 13:55:07 2024 +1000)
* b50488d - chore: Get musl build to work on latest Rust (1.77.1) (Ronald Holshausen, Tue Apr 9 13:38:19 2024 +1000)
* 0b2cf00 - chore: Fix musl build (Ronald Holshausen, Tue Apr 9 12:42:27 2024 +1000)
* 5aacf85 - chore: support musl build with Rust 1.77.1 (Ronald Holshausen, Tue Apr 9 11:56:36 2024 +1000)
* bdb815c - chore: Update dependencies (Ronald Holshausen, Tue Apr 9 10:58:21 2024 +1000)
* a88de0c - chore: Lock ahash crate as 0.8.8 requires Rust 1.72 (Ronald Holshausen, Tue Feb 13 11:20:36 2024 +1100)
* 067394d - chore: Downgrade ahash crate as 0.8.8 requires Rust 1.72 (Ronald Holshausen, Tue Feb 13 10:03:52 2024 +1100)
* 6751c74 - chore: Lock clap crate to 4.4 as 4.5 requires Rust 1.75 (Ronald Holshausen, Mon Feb 12 16:51:20 2024 +1100)
* 3aee270 - chore: Updated integrated_tests/response_metadata to use the corrected version of pact_consumer (Ronald Holshausen, Mon Feb 12 15:56:45 2024 +1100)
* 4d4aa78 - test: integrated test for response metadata (Stan, Thu Feb 8 17:12:10 2024 -0800)
* 7dbf946 - chore: Cleanup clippy warnings (Ronald Holshausen, Wed Feb 7 23:21:05 2024 +1100)
* 4a6772f - chore: Cleanup clippy warnings (Ronald Holshausen, Wed Feb 7 23:09:47 2024 +1100)
* 76067b2 - fix: Support configuring primitve values for fields with native values instead of strings (Ronald Holshausen, Wed Feb 7 22:59:33 2024 +1100)
* 1e4b8eb - bump version to 0.3.14 (Ronald Holshausen, Wed Feb 7 17:26:41 2024 +1100)

# 0.3.13 - Bugfix Release

* 341d4d7 - fix: do not inject a default value for repeated fields #45 (Ronald Holshausen, Wed Feb 7 16:13:59 2024 +1100)
* 7674f5e - bump version to 0.3.13 (Ronald Holshausen, Wed Feb 7 13:57:26 2024 +1100)

# 0.3.12 - Bugfix Release

* 131ffc4 - doc: Add docs on matching on maps to the README (Ronald Holshausen, Wed Feb 7 13:52:32 2024 +1100)
* 3f369a9 - fix: Correct the use of matching rules on maps (Ronald Holshausen, Wed Feb 7 13:43:02 2024 +1100)
* 9d88e92 - feat: Support defining an each values matcher for a whole message (Ronald Holshausen, Tue Feb 6 16:31:24 2024 +1100)
* 002c9d6 - chore: only run clippy on linux CI agent (Ronald Holshausen, Tue Feb 6 11:01:43 2024 +1100)
* eb0acc5 - chore: Add integrated test examples to the CI build (Ronald Holshausen, Tue Feb 6 10:53:48 2024 +1100)
* 0a6458c - fix: accept empty maps where there is a eachKey or eachValue matcher (Ronald Holshausen, Tue Feb 6 10:40:24 2024 +1100)
* c81187c - chore: Update dependencies (Ronald Holshausen, Tue Feb 6 09:59:28 2024 +1100)
* 7d80aba - bump version to 0.3.12 (Ronald Holshausen, Tue Jan 30 11:02:22 2024 +1100)

# 0.3.11 - Bugfix Release

* 4db4328 - fix: when checking for unexpected fields, ignore fields with default values (Ronald Holshausen, Tue Jan 30 10:15:18 2024 +1100)
* b32bdb6 - bump version to 0.3.11 (Ronald Holshausen, Mon Jan 29 16:43:23 2024 +1100)

# 0.3.10 - Maintenance Release

* 2aa8016 - chore: Update flaky mock server test (Ronald Holshausen, Mon Jan 29 15:50:16 2024 +1100)
* ddd2449 - chore: Upgrade prost and tonic to latest versions (Ronald Holshausen, Sun Jan 21 07:03:38 2024 +1100)
* 661a154 - fix: Missing fields may have been dropped from the payload if they had a default value (Ronald Holshausen, Sun Jan 21 06:52:07 2024 +1100)
* 5b665a6 - bump version to 0.3.10 (Ronald Holshausen, Sat Jan 20 07:04:33 2024 +1100)

# 0.3.9 - Bugfix Release

* 216c13f - chore: Upgrade dependencies (Ronald Holshausen, Sat Jan 20 06:48:00 2024 +1100)
* 08599f2 - chore: Add a test for merging between plugin config and manifest values #41 (Ronald Holshausen, Sat Jan 20 05:46:17 2024 +1100)
* 3c51f8d - Merge pull request #42 from rkrishnan2012/main (Ronald Holshausen, Tue Jan 23 15:35:02 2024 +1100)
* 884b36c - fix: Repeated enum fields must be encoded as packed varints #27 (Ronald Holshausen, Sat Jan 20 04:33:05 2024 +1100)
* 6f9844c - Fix merging between plugin config and manifest values. (Rohit Krishnan, Fri Jan 19 10:32:27 2024 -0500)
* 9caef2a - chore: Add example test with a repeated enum field #27 (Ronald Holshausen, Sat Jan 20 02:20:10 2024 +1100)
* b5881d7 - bump version to 0.3.9 (Ronald Holshausen, Sat Dec 16 22:03:53 2023 +1100)

# 0.3.8 - Bugfix Release

* 97113ab - chore: Upgrade all dependencies (Ronald Holshausen, Sat Dec 16 19:57:28 2023 +1100)
* af9657f - fix: correct URL for aarch64 macs. Fixes #39 (Stan, Thu Dec 14 13:07:12 2023 -0800)
* 0212f56 - chore: Use cross from GitHub for building aarch64 target for release (Ronald Holshausen, Wed Nov 8 17:17:11 2023 +1100)
* 12c98c5 - chore: revert Cargo lock update (Ronald Holshausen, Sat Nov 4 15:34:36 2023 +1100)
* 96ed737 - chore: rename step in CI (Ronald Holshausen, Sat Nov 4 15:25:43 2023 +1100)
* d678376 - bump version to 0.3.8 (Ronald Holshausen, Sat Nov 4 15:25:17 2023 +1100)
* 71333ba - update changelog for release 0.3.7 (Ronald Holshausen, Sat Nov 4 15:24:52 2023 +1100)
* 27af477 - chore: fix black box tests in CI (Ronald Holshausen, Sat Nov 4 15:06:53 2023 +1100)
* 6c626dd - chore: Black box tests require the plugin to be built first (Ronald Holshausen, Sat Nov 4 13:45:59 2023 +1100)
* 12fab8e - chore: Black box tests require the plugin to be installed (Ronald Holshausen, Sat Nov 4 13:32:43 2023 +1100)
* b04dc47 - chore: Integration (black box) tests were not being run in CI (Ronald Holshausen, Sat Nov 4 13:19:06 2023 +1100)
* 0a540eb - fix nested enum not resolving (zsylvia, Thu Nov 2 17:18:51 2023 -0400)
* 8bea782 - bump version to 0.3.7 (Ronald Holshausen, Thu Sep 21 08:29:45 2023 +1000)

# 0.3.7 - Bugfix Release

* 27af477 - chore: fix black box tests in CI (Ronald Holshausen, Sat Nov 4 15:06:53 2023 +1100)
* 6c626dd - chore: Black box tests require the plugin to be built first (Ronald Holshausen, Sat Nov 4 13:45:59 2023 +1100)
* 12fab8e - chore: Black box tests require the plugin to be installed (Ronald Holshausen, Sat Nov 4 13:32:43 2023 +1100)
* b04dc47 - chore: Integration (black box) tests were not being run in CI (Ronald Holshausen, Sat Nov 4 13:19:06 2023 +1100)
* 0a540eb - fix nested enum not resolving (zsylvia, Thu Nov 2 17:18:51 2023 -0400)
* 8bea782 - bump version to 0.3.7 (Ronald Holshausen, Thu Sep 21 08:29:45 2023 +1000)

# 0.3.6 - Bugfix Release

* 2731ee4 - Fix bug resolving enum across multiple files. (Rohit Krishnan, Mon Sep 18 16:06:06 2023 -0400)
* d5fcc14 - bump version to 0.3.6 (Ronald Holshausen, Wed Aug 9 14:36:23 2023 +1000)

# 0.3.5 - Bugfix Release

* fe47ce2 - fix: Corrected processing of Map fields to also support primitive values (Ronald Holshausen, Wed Aug 9 14:33:39 2023 +1000)
* 80c8b75 - chore: Update all dependencies (Ronald Holshausen, Wed Aug 9 11:24:59 2023 +1000)
* fb22bfb - bump version to 0.3.5 (Ronald Holshausen, Thu Jun 22 16:20:37 2023 +1000)

# 0.3.4 - Bugfix Release

* 098903e - fix: EachValue matcher was not applying regex to repeated fields correctly #22 (Ronald Holshausen, Thu Jun 22 16:06:29 2023 +1000)
* 7ec725a - chore: Run aarch64 build with 1.69 rust as 1.70 fails with a gcc link error (Ronald Holshausen, Tue Jun 20 13:32:45 2023 +1000)
* cfcd63e - bump version to 0.3.4 (Ronald Holshausen, Tue Jun 20 12:03:43 2023 +1000)

# 0.3.3 - Bugfix Release

* b6fd769 - fix: correct invalid matchig rule path when using each value with a reference #22 (Ronald Holshausen, Tue Jun 20 11:45:51 2023 +1000)
* 40c4ea3 - chore: add simple enum test with repeated field #27 (Ronald Holshausen, Wed Jun 7 16:21:27 2023 +1000)
* 5a1abd8 - chore: cleanup some deprecation warnings (Ronald Holshausen, Wed Jun 7 10:15:39 2023 +1000)
* 428a6ae - chore: lock cross to previous version as latest fails on GH (Ronald Holshausen, Wed Jun 7 10:12:22 2023 +1000)
* 688fb69 - chore: correct release script (Ronald Holshausen, Wed Jun 7 09:16:42 2023 +1000)
* f703e0e - chore: bump version to 0.3.3 (Ronald Holshausen, Wed Jun 7 09:15:46 2023 +1000)

# 0.3.2 - Bugfix Release

* a0726e2 - fix: incorrect the matching rules where setup when an EachValues matcher was used with a repeated field #22 (Ronald Holshausen, Tue Jun 6 16:48:53 2023 +1000)
* e0b3084 - bump version to 0.3.2 (Ronald Holshausen, Mon Jun 5 16:32:19 2023 +1000)

# 0.3.1 - Support gRPC error responses + bugfixes

* d1f560f - chore: fix typo in readme (Ronald Holshausen, Mon Jun 5 16:21:18 2023 +1000)
* c3ed9fa - chore: add section on Verifying gRPC error responses to readme (Ronald Holshausen, Mon Jun 5 16:19:31 2023 +1000)
* 4b2c839 - fix: fix for "Did not find interaction with key XXXXX in the Pact" error (Ronald Holshausen, Mon Jun 5 15:37:23 2023 +1000)
* 9b7ec93 - fix: correct incorrect status in the output from the metadata comparison results (Ronald Holshausen, Mon Jun 5 15:34:56 2023 +1000)
* 8c57a04 - chore: Update dependencies (Ronald Holshausen, Mon Jun 5 15:33:43 2023 +1000)
* ce32874 - feat: Support validating metadata when an error response is returned (Ronald Holshausen, Fri Jun 2 15:48:15 2023 +1000)
* 564cefa - feat: Support setting the gRPC status for the response (Ronald Holshausen, Fri Jun 2 09:29:07 2023 +1000)
* b25b4aa - chore: Upgrade all dependencies (Ronald Holshausen, Thu Jun 1 10:39:57 2023 +1000)
* 46b7b23 - chore: add smartbear supported workflow (Matt Fellows, Sun May 21 19:09:32 2023 +1000)
* 9c07576 - chore: Upgrade dependencies (Ronald Holshausen, Wed Apr 5 11:53:02 2023 +1000)
* 8cb6c4f - fix: manifest had the wrong version (Ronald Holshausen, Wed Feb 22 09:03:12 2023 +1100)
* 8ded034 - chore: update readme (Ronald Holshausen, Fri Feb 10 14:06:56 2023 +1100)
* 5278a01 - bump version to 0.3.1 (Ronald Holshausen, Fri Feb 10 14:03:51 2023 +1100)

# 0.3.0 - Supports gRPC metadata with Pact tests

* cab8ce1 - fix: Upgrade pact_verifier crate to 0.13.21 (fixes pact verification test) (Ronald Holshausen, Fri Feb 10 13:40:48 2023 +1100)
* b0e3bf9 - feat: implement validating response metadata (Ronald Holshausen, Thu Feb 9 18:04:23 2023 +1100)
* 29dfb4a - fix: gRPC response metadata was not being processed correctly (Ronald Holshausen, Thu Feb 9 14:48:52 2023 +1100)
* dc78c0c - feat: support validation and metadata in consumer tests (Ronald Holshausen, Wed Feb 8 16:05:38 2023 +1100)
* 024e8d3 - feat: support configuring gRPC message metadata in consumer tests (Ronald Holshausen, Mon Feb 6 15:50:01 2023 +1100)
* b2e8614 - chore: Upgrade clap to v4 (Ronald Holshausen, Fri Feb 3 15:21:44 2023 +1100)
* 5aa7089 - chore: Upgrade base64 crate (Ronald Holshausen, Fri Feb 3 14:47:44 2023 +1100)
* a702647 - chore: bump minor version (Ronald Holshausen, Fri Feb 3 14:02:52 2023 +1100)
* 88ce08e - bump version to 0.2.6 (Ronald Holshausen, Fri Feb 3 13:25:02 2023 +1100)

# 0.2.5 - Bugfix Release

* 0f44247 - fix: Handle interactions with empty messages (Ronald Holshausen, Fri Feb 3 10:37:06 2023 +1100)
* 8734416 - chore: Update dependencies (Ronald Holshausen, Fri Feb 3 10:35:28 2023 +1100)
* 22eb0cf - bump version to 0.2.5 (Ronald Holshausen, Wed Dec 21 15:35:28 2022 +1100)

# 0.2.4 - support passing in protoc config from the consumer test

* 834a8e3 - Update README.md (Ronald Holshausen, Wed Dec 21 15:22:36 2022 +1100)
* 6c82795 - chore: Update readme with specifying configuration values in the tests (Ronald Holshausen, Wed Dec 21 15:20:49 2022 +1100)
* 79ba606 - feat: support passing in protoc config from the consumer test (Ronald Holshausen, Wed Dec 21 14:40:56 2022 +1100)
* 18f7fff - Create issue-comment-created.yml (Ronald Holshausen, Wed Dec 21 10:32:55 2022 +1100)
* 95a48f7 - bump version to 0.2.4 (Ronald Holshausen, Mon Dec 19 15:12:14 2022 +1100)

# 0.2.3 - Support Generators

* 7f0e9d7 - feat: Implemented RandomBoolean, ProviderStateGenerator and MockServerURL generators (Ronald Holshausen, Mon Dec 19 14:53:05 2022 +1100)
* 2b2978e - feat: Implemented RandomHexadecimal, RandomString and Regex generators (Ronald Holshausen, Mon Dec 19 14:17:49 2022 +1100)
* 2804ad5 - feat: Implemented RandomInt, RandomDecimal and Uuid generators (Ronald Holshausen, Mon Dec 19 13:58:20 2022 +1100)
* 9502770 - refactor: move generator code to its own module (Ronald Holshausen, Mon Dec 19 13:11:36 2022 +1100)
* f796ad3 - chore: fix clippy warnings (Ronald Holshausen, Mon Dec 19 11:04:09 2022 +1100)
* fc159a7 - feat: support adding a test context to the mock server to support generators (Ronald Holshausen, Fri Dec 16 17:24:15 2022 +1100)
* ac210b1 - feat: Support using generators with mock server responses (Ronald Holshausen, Fri Dec 16 15:36:33 2022 +1100)
* 5024b82 - bump version to 0.2.3 (Ronald Holshausen, Thu Dec 15 16:28:05 2022 +1100)

# 0.2.2 - Fix for broken 0.2.1 release

* e5d129f - bump version to 0.2.2 (Ronald Holshausen, Wed Dec 14 17:32:36 2022 +1100)
* 0b2ab5a - update changelog for release 0.2.1 (Ronald Holshausen, Wed Dec 14 17:32:26 2022 +1100)
* 80ec6c0 - feat: add partial support for generators with Protobufs (date/time only) (Ronald Holshausen, Wed Dec 14 17:20:38 2022 +1100)
* 90e90d7 - fix: set the protoc command and error logs to better levels (Ronald Holshausen, Fri Dec 9 16:00:43 2022 +1100)
* 011af5b - bump version to 0.2.1 (Ronald Holshausen, Fri Nov 25 14:03:28 2022 +1100)
* ec91ea7 - chore: unlock http-body (Ronald Holshausen, Thu Dec 15 16:08:16 2022 +1100)
* bb0259f - chore: upgrade tonic, prost, hyper, tower (Ronald Holshausen, Thu Dec 15 16:05:12 2022 +1100)
* 1786205 - chore: update dependencies (Ronald Holshausen, Thu Dec 15 15:57:35 2022 +1100)
* 50ecd70 - chore: lock tracing to previous version (Ronald Holshausen, Thu Dec 15 15:55:10 2022 +1100)
* 2a85da3 - chore: lock tower to previous version (Ronald Holshausen, Thu Dec 15 15:51:47 2022 +1100)
* 9324f18 - chore: lock prost, pact, hyper to previous version (Ronald Holshausen, Thu Dec 15 15:49:45 2022 +1100)
* 02b292c - chore: lock tonic to previous version (Ronald Holshausen, Thu Dec 15 15:43:46 2022 +1100)

# 0.2.1 - Support Date/Time generators

* b6be68d - feat: add partial support for generators with Protobufs (date/time only) (Ronald Holshausen, Wed Dec 14 17:20:38 2022 +1100)
* ec3dd88 - fix: set the protoc command and error logs to better levels (Ronald Holshausen, Fri Dec 9 16:00:43 2022 +1100)
* f992968 - bump version to 0.2.1 (Ronald Holshausen, Fri Nov 25 14:03:28 2022 +1100)

# 0.2.0 - Bugfix Release

* 04cb32d - chore: Bump minor version (Ronald Holshausen, Fri Nov 25 11:20:21 2022 +1100)
* 839650b - feat: add an integration test for mock server not getting any requests #15 (Ronald Holshausen, Wed Nov 23 18:15:18 2022 +1100)
* 75d7343 - fix(mock-server): return an error if we don't get a request for any service method #15 (Ronald Holshausen, Wed Nov 23 16:47:14 2022 +1100)
* 62cf24f - chore: Upgrade the Pact libs (Ronald Holshausen, Tue Nov 22 17:10:26 2022 +1100)
* 2868bc7 - chore: upgrade dependencies (Ronald Holshausen, Tue Nov 22 16:57:46 2022 +1100)
* 287cf89 - bump version to 0.1.18 (Ronald Holshausen, Thu Nov 17 11:21:52 2022 +1100)

# 0.1.17 - Bugfix Release

* 45e67e3 - fix: was not finding enums where the package had more than one element in the path (Ronald Holshausen, Thu Nov 17 11:02:14 2022 +1100)
* 8b02d88 - fix: revert the pact crate update as it was breaking access via FFI (Ronald Holshausen, Thu Nov 17 10:36:06 2022 +1100)
* ab326e4 - bump version to 0.1.17 (Ronald Holshausen, Thu Nov 10 14:56:19 2022 +1100)

# 0.1.16 - Bugfix Release

* dbacfa4 - fix: was not finding enums where the package had more than one element in the path (Ronald Holshausen, Thu Nov 10 14:35:02 2022 +1100)
* 23a9a92 - bump version to 0.1.16 (Ronald Holshausen, Wed Oct 5 13:48:07 2022 +1100)

# 0.1.15 - Bugfix Release

* 8eb7d55 - chore: add a test with message types from an imported proto file #11 (Ronald Holshausen, Wed Oct 5 13:43:35 2022 +1100)
* 9ebdf4f - fix: Suppport message types embedded in other message types (Ronald Holshausen, Wed Oct 5 12:14:54 2022 +1100)
* 473f61b - fix: support message fields with global enum values (not local to a message) (Ronald Holshausen, Tue Oct 4 18:14:27 2022 +1100)
* 644dcc1 - bump version to 0.1.15 (Ronald Holshausen, Tue Sep 20 15:59:29 2022 +1000)

# 0.1.14 - Bugfix Release

* 127a318 - chore: cleanup compiler warnings (Ronald Holshausen, Tue Sep 20 15:56:59 2022 +1000)
* 59a1bd2 - fix: Support matching on fields that are defined in imported proto files (Ronald Holshausen, Tue Sep 20 15:38:40 2022 +1000)
* 0c37f1b - fix: errors configuring request fields were being swallowed (Ronald Holshausen, Tue Sep 20 12:26:06 2022 +1000)
* 8d8cc90 - bump version to 0.1.14 (Ronald Holshausen, Mon Sep 12 12:09:31 2022 +1000)

# 0.1.13 - Bugfix Release

* 493f476 - chore: cleanup some compiler messages (Ronald Holshausen, Mon Sep 12 12:07:44 2022 +1000)
* 31a8873 - fix: Generate the correct matching rule paths for repeated fields (Ronald Holshausen, Mon Sep 12 11:55:56 2022 +1000)
* 2d53f90 - chore: update readme with installation instructions (Ronald Holshausen, Thu Aug 25 14:02:18 2022 +1000)
* 6d7da55 - bump version to 0.1.13 (Ronald Holshausen, Thu Aug 25 12:07:34 2022 +1000)

# 0.1.12 - Fix for repeated fields

* 1e1080c - chore: cleaned up some compiler warnings (Ronald Holshausen, Thu Aug 25 11:44:11 2022 +1000)
* 5abf1eb - chore: update dependencies (Ronald Holshausen, Thu Aug 25 10:35:23 2022 +1000)
* 874b362 - fix: matching rule paths for repeated fields were not correct when configured with data in a map form (Ronald Holshausen, Wed Aug 24 17:37:25 2022 +1000)
* ae823c0 - fix: matching rule paths for repeated fields were not correct when configured with an each value matcher (Ronald Holshausen, Wed Aug 24 17:32:04 2022 +1000)
* 9d896af - fix: ensure there is the enough bytes to read a repeated packed field (Ronald Holshausen, Tue Aug 23 16:44:49 2022 +1000)
* 7773c99 - feat: support decoding packed repeated fields (Ronald Holshausen, Tue Aug 23 16:22:32 2022 +1000)
* c65deb4 - feat: support packed repeated fields (Ronald Holshausen, Tue Aug 23 14:32:02 2022 +1000)
* 08b7b7e - chore: add github token to avoid throttle errors installing protoc (Ronald Holshausen, Wed Aug 17 14:25:38 2022 +1000)
* a266c64 - Revert "Revert "bump version to 0.1.12"" (Ronald Holshausen, Wed Aug 17 13:52:57 2022 +1000)

# 0.1.11 - Support google.protobuf.StringValue and repeated fields configured with lists of values

* d7b37b0 - fix: return an error if any of the response parts fail to be parsed or constructored (Ronald Holshausen, Wed Aug 17 11:58:50 2022 +1000)
* 1fea4f4 - fix: Support repeated fields configured with lists of values (Ronald Holshausen, Wed Aug 17 11:50:41 2022 +1000)
* 5417017 - chore: add protoc to the build (Ronald Holshausen, Tue Aug 16 17:01:34 2022 +1000)
* f5bc948 - chore: Upgrade Pact, Tonic and Prost crates (Ronald Holshausen, Tue Aug 16 16:35:19 2022 +1000)
* fe99f4b - chore: add arm64 Linux target to the release build (Ronald Holshausen, Tue Aug 16 15:19:09 2022 +1000)
* 73b3c2f - chore: fix Alpine build (Ronald Holshausen, Tue Aug 16 15:04:55 2022 +1000)
* 35e8046 - chore: fix Alpine build (Ronald Holshausen, Tue Aug 16 14:44:49 2022 +1000)
* e77ec3f - chore: Update dependencies (Ronald Holshausen, Tue Aug 16 14:06:38 2022 +1000)
* a54dcaf - fix: Support using google.protobuf.StringValue with service method calls (Ronald Holshausen, Tue Aug 16 13:37:32 2022 +1000)
* 0f5b291 - Merge pull request #7 from pactflow/whitesource/configure (Ronald Holshausen, Tue Aug 9 17:12:42 2022 +1000)
* 7c137d9 - bump version to 0.1.11 (Ronald Holshausen, Tue Aug 9 17:03:56 2022 +1000)
* cea4fc0 - Add .whitesource configuration file (mend-for-github-com[bot], Tue Jul 19 20:17:00 2022 +0000)

# 0.1.10 - Maintenance Release

* 4c83a29 - feat: correct the trace logging of protoc command (Ronald Holshausen, Tue Aug 9 16:35:04 2022 +1000)
* ae8fb4b - feat: allow additional includes to be configured for protoc (Ronald Holshausen, Tue Aug 9 15:15:56 2022 +1000)
* a16cd9c - chore: add configuration section to readme (Ronald Holshausen, Tue Aug 9 14:22:13 2022 +1000)
* 2faf8f4 - fix(Windows): correct the protobuf include for Windows (Ronald Holshausen, Tue Aug 9 13:53:35 2022 +1000)
* 653cc79 - feat: default the address to bind to to the IP4 lookback adapter (Ronald Holshausen, Mon Aug 8 17:32:17 2022 +1000)
* 6df9505 - bump version to 0.1.10 (Ronald Holshausen, Mon Aug 8 16:53:43 2022 +1000)

# 0.1.9 - build linux binary with musl

* 30ce99e - chore: Update dependencies (Ronald Holshausen, Mon Aug 8 16:51:44 2022 +1000)
* 5590dbc - feat: build linux binary with musl (Ronald Holshausen, Mon Aug 8 16:47:15 2022 +1000)
* 6996dd2 - bump version to 0.1.9 (Ronald Holshausen, Fri Jul 15 13:17:25 2022 -0400)

# 0.1.8 - Support IP4 addresses in docker

* ae7a7e3 - fix: Update the readme with host parameter details (Ronald Holshausen, Wed Jul 13 15:31:17 2022 -0400)
* 1512f96 - fix: add host parameter to support IP4 adresses in docker (Ronald Holshausen, Wed Jul 13 15:16:37 2022 -0400)
* 1d57ede - chore: Upgrade all the pact crates to latest versions (Ronald Holshausen, Mon May 30 12:35:17 2022 +1000)
* 183cc80 - bump version to 0.1.8 (Ronald Holshausen, Mon May 30 11:12:58 2022 +1000)

# 0.1.7 - Bugfix Release

* 9697164 - fix: need to consider the default values when comparing with a missing field value (Ronald Holshausen, Fri May 27 16:02:49 2022 +1000)
* ad9c37b - chore: update the tracing events for matching payloads (Ronald Holshausen, Fri May 27 10:35:07 2022 +1000)
* 8dc4c17 - fix: disable ansi mode so the log file is more readable (Ronald Holshausen, Thu May 26 14:19:17 2022 +1000)
* 820613d - chore: Upgrade to pact-plugin-driver 0.1.8 (Ronald Holshausen, Thu May 26 14:18:43 2022 +1000)
* 6a12675 - chore: no point logging that you can not install logging (Ronald Holshausen, Wed May 25 14:19:39 2022 +1000)
* 11ddc11 - bump version to 0.1.7 (Ronald Holshausen, Wed May 25 13:37:30 2022 +1000)

# 0.1.6 - Bugfix Release

* 580baba - fix: do not shutdown server for a get_mock_server_results request (Ronald Holshausen, Tue May 24 17:04:02 2022 +1000)
* 0a3cb5f - feat: implement method for mock server results for FFI functions (Ronald Holshausen, Tue May 24 16:44:15 2022 +1000)
* 009efa0 - chore: add the install script to the release build (Ronald Holshausen, Tue May 24 16:29:14 2022 +1000)
* 2f4556b - chore: correct the install plugin script (Ronald Holshausen, Tue May 24 11:51:17 2022 +1000)
* 593dc63 - core: add bash script to install plugin (Ronald Holshausen, Tue May 24 10:25:22 2022 +1000)
* b998318 - fix: correct the installation docs to make the plugin executable (Ronald Holshausen, Mon May 16 14:50:55 2022 +1000)
* 8f7956c - fix: correct the installation docs to make the plugin executable (Ronald Holshausen, Mon May 16 14:49:50 2022 +1000)
* eb38ca2 - chore: fix pact test after upgrading deps (Ronald Holshausen, Tue May 10 13:56:08 2022 +1000)
* 4a82af7 - bump version to 0.1.6 (Ronald Holshausen, Tue May 10 13:26:53 2022 +1000)

# 0.1.5 - Updated logging

* 6b3c0ca - chore: update readme (Ronald Holshausen, Tue May 10 12:10:00 2022 +1000)
* 4ec5788 - chore: cleanup unused imports (Ronald Holshausen, Tue May 10 11:58:22 2022 +1000)
* 55c9fa5 - feat: add bunyan formatted JSON logs (Ronald Holshausen, Tue May 10 11:44:46 2022 +1000)
* 3dae582 - chore: fix failing CI build (Ronald Holshausen, Tue May 10 11:14:40 2022 +1000)
* 905b19e - feat: use tracing appender for a rolling log file instead of log4rs (Ronald Holshausen, Tue May 10 10:46:45 2022 +1000)
* 5e547cf - bump version to 0.1.5 (Ronald Holshausen, Thu May 5 13:26:06 2022 +1000)

# 0.1.4 - Bugfix Release

* 45a9937 - fix(windows): Protoc does not support Windows paths that start with \\?\ (Ronald Holshausen, Thu May 5 11:41:13 2022 +1000)
* f1e14fc - fix(windows): Use native OS paths when execting protoc binary (Ronald Holshausen, Wed May 4 17:25:43 2022 +1000)
* 997be06 - chore: update readme with gRPC support (Ronald Holshausen, Fri Apr 29 15:05:22 2022 +1000)
* 071e150 - bump version to 0.1.4 (Ronald Holshausen, Fri Apr 29 09:58:27 2022 +1000)

# 0.1.3 - Updated verification output

* 57f7cbd - chore: fix the CI build (Ronald Holshausen, Fri Apr 29 09:22:44 2022 +1000)
* 6f22d4e - chore: update dependencies (Ronald Holshausen, Fri Apr 29 09:14:53 2022 +1000)
* 070ebbc - feat: add verification output for the verification call (Ronald Holshausen, Tue Apr 26 16:50:57 2022 +1000)
* ab7d0d8 - bump version to 0.1.3 (Ronald Holshausen, Tue Apr 12 15:59:17 2022 +1000)

# 0.1.2 - Bugfix Release

* accda0d - feat: add a shutdown time of 10 minutes to avoid hanging processes (Ronald Holshausen, Tue Apr 12 15:22:38 2022 +1000)
* fda0844 - fix(regression): gRPC implementaton broke verifying Protobuf messages (Ronald Holshausen, Tue Apr 12 12:45:52 2022 +1000)
* 9993693 - chore: debugging CI build (Ronald Holshausen, Tue Apr 12 11:08:04 2022 +1000)
* b319205 - bump version to 0.1.2 (Ronald Holshausen, Mon Apr 11 18:06:20 2022 +1000)

# 0.1.1 - Support verifying gRPC requests

* d42a5c7 - chore: use the published version of pact-plugin-driver (Ronald Holshausen, Mon Apr 11 17:40:04 2022 +1000)
* 0f2d989 - Revert "update changelog for release 0.1.1" (Ronald Holshausen, Mon Apr 11 17:25:10 2022 +1000)
* b4f59eb - update changelog for release 0.1.1 (Ronald Holshausen, Mon Apr 11 17:22:23 2022 +1000)
* d88641e - fix: add the wire type into the failure message (Ronald Holshausen, Mon Apr 11 14:41:13 2022 +1000)
* f0cd56e - fix: handle case where actual field does not match expected discriptor (Ronald Holshausen, Mon Apr 11 14:36:41 2022 +1000)
* 8065bff - fix: handle additional fields from the provider (Ronald Holshausen, Mon Apr 11 13:57:16 2022 +1000)
* 453a215 - chore: fix clippy violations (Ronald Holshausen, Mon Apr 11 11:36:27 2022 +1000)
* f33a8ae - chore: cleanup compiler messages (Ronald Holshausen, Mon Apr 11 11:13:56 2022 +1000)
* 5a4915e - feat: initial attempt at verifcation (Ronald Holshausen, Fri Apr 8 14:30:27 2022 +1000)
* 377f010 - feat: Initial gRPC request implementation for verifying (Ronald Holshausen, Thu Apr 7 12:57:45 2022 +1000)
* 1a907fa - feat: implemented the plubming for verifing requests (Ronald Holshausen, Wed Apr 6 09:17:15 2022 +1000)
* 66e0f38 - bump version to 0.1.1 (Ronald Holshausen, Thu Mar 24 17:02:48 2022 +1100)

# 0.1.0 - gRPC mock servers

* f49c15a - chore: clean in prep for release (Ronald Holshausen, Thu Mar 24 16:09:09 2022 +1100)
* 8df497c - chore: add the gRPC service name into any error messages from the mock server (Ronald Holshausen, Thu Mar 17 16:29:03 2022 +1100)
* f77b773 - chore: bind to IP6 loopback address (Ronald Holshausen, Thu Mar 17 10:50:54 2022 +1100)
* 2bfcd37 - fix: matching rule paths were incorrect for gRPC interactions (Ronald Holshausen, Tue Mar 15 14:00:48 2022 +1100)
* 6c2f38b - feat: return the results back from the mock server (Ronald Holshausen, Fri Mar 11 16:36:53 2022 +1100)
* fef6b67 - chore: cleanup unused imports (Ronald Holshausen, Wed Mar 9 15:04:31 2022 +1100)
* cfe24a8 - feat: first working version of a gRPC mock server (Ronald Holshausen, Wed Mar 9 14:34:42 2022 +1100)
* d66ace5 - feat: Initial setup of basic mock gRPC server (Ronald Holshausen, Mon Mar 7 15:19:33 2022 +1100)
* d782063 - Merge branch 'main' into feat/mock-server (Ronald Holshausen, Mon Mar 7 10:39:31 2022 +1100)
* e7eebc7 - bump version to 0.0.4 (Ronald Holshausen, Mon Feb 28 10:35:41 2022 +1100)
* 897eae2 - chore: bump minor version (Ronald Holshausen, Wed Feb 2 15:04:49 2022 +1100)

# 0.0.3 - Bugfix Release

* 34d0248 - fix: check not empty value for unexpected keys (tienvx, Tue Feb 1 10:41:33 2022 +0700)
* aa49c13 - fix: extract message type from input type to compare (tienvx, Sat Jan 29 15:44:11 2022 +0700)
* 59c0c44 - bump version to 0.0.3 (Ronald Holshausen, Tue Jan 25 16:43:22 2022 +1100)

# 0.0.2 - Fix for interactions over HTTP

* 7040854 - chore: add Rust version to readme (Ronald Holshausen, Tue Jan 25 16:04:09 2022 +1100)
* 29803ec - fix: for interactions over HTTP, need to specify if the message is for the request or response (Ronald Holshausen, Tue Jan 25 15:29:11 2022 +1100)
* 9283375 - chore: Update crates (Ronald Holshausen, Tue Jan 25 11:54:50 2022 +1100)
* 5627fac - fix: print correct message in debug log (tienvo, Sat Jan 22 00:10:01 2022 +0700)
* 49cdead - chore: Upgrade pavy-models, pact-matching and plugin driver crates (Ronald Holshausen, Mon Jan 17 11:35:35 2022 +1100)
* 05c4df9 - chore: update readme (Ronald Holshausen, Fri Jan 14 14:12:44 2022 +1100)
* 8b08746 - chore: update readme (Ronald Holshausen, Fri Jan 14 14:10:42 2022 +1100)
* faa350c - chore: update release script (Ronald Holshausen, Fri Jan 14 10:47:45 2022 +1100)
* 2dd98a9 - bump version to 0.0.2 (Ronald Holshausen, Fri Jan 14 10:45:27 2022 +1100)

# 0.0.1 - configurable logging

* c5a09ce - feat: add configurable logging, default logging to also write to a file (Ronald Holshausen, Thu Jan 13 17:44:09 2022 +1100)
* 9ed4b00 - chore: update readme (Ronald Holshausen, Wed Jan 5 11:47:22 2022 +1100)
* e966520 - chore: update readme (Ronald Holshausen, Wed Jan 5 11:37:03 2022 +1100)
* f54b575 - chore: update readme (Ronald Holshausen, Wed Jan 5 10:12:21 2022 +1100)
* b8598ec - chore: update readme (Ronald Holshausen, Tue Jan 4 16:54:55 2022 +1100)
* a02903f - chore: update readme (Ronald Holshausen, Tue Jan 4 16:23:59 2022 +1100)
* 32ebe6a - chore: update readme (Ronald Holshausen, Tue Jan 4 15:56:02 2022 +1100)
* 8780e10 - chore: update readme (Ronald Holshausen, Tue Jan 4 14:39:18 2022 +1100)
* 5ae876c - chore: update readme (Ronald Holshausen, Tue Jan 4 14:27:12 2022 +1100)
* fa5288e - chore: add readme (Ronald Holshausen, Tue Jan 4 13:48:57 2022 +1100)
* 1a9ffd7 - chore: fix pact_matching to the githib version (Ronald Holshausen, Tue Jan 4 13:22:38 2022 +1100)
* 5720d98 - chore: update plugin driver to 0.0.16 and pact verifier to 0.12.2 (Ronald Holshausen, Tue Jan 4 12:15:43 2022 +1100)
* 8c3a54f - chore: Upgrade pact_verifier to 0.12.2 (Ronald Holshausen, Fri Dec 31 15:37:27 2021 +1100)
* 3f97ae8 - fix: update pact-plugin-driver to 0.0.15 (fixes issue with version) (Ronald Holshausen, Fri Dec 31 15:17:25 2021 +1100)
* 6bbf654 - chore: Update manifest file for release (Ronald Holshausen, Fri Dec 31 12:42:15 2021 +1100)
* 1176114 - bump version to 0.0.1 (Ronald Holshausen, Fri Dec 31 12:22:39 2021 +1100)

# 0.0.0 - First Release
