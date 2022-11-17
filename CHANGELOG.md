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
