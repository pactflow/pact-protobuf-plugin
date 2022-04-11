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
