# Changelog

## [0.12.0](https://github.com/hugoocoto/runtime-arena/compare/v0.11.1...v0.12.0) (2026-11-26)


### Features

* **admin:** clone rounds into new drafts ([#71](https://github.com/hugoocoto/runtime-arena/issues/71)) ([6afe029](https://github.com/hugoocoto/runtime-arena/commit/6afe029a1f1d5a40bb5d7863d8977117cc2c28cf)), closes [#64](https://github.com/hugoocoto/runtime-arena/issues/64)
* **cli:** add --once and --json snapshots to arena board ([#66](https://github.com/hugoocoto/runtime-arena/issues/66)) ([5cc1eff](https://github.com/hugoocoto/runtime-arena/commit/5cc1effbf21da8371a9cbbbc4e1ea6bdd9164a68)), closes [#39](https://github.com/hugoocoto/runtime-arena/issues/39) [#40](https://github.com/hugoocoto/runtime-arena/issues/40) [#48](https://github.com/hugoocoto/runtime-arena/issues/48) [#56](https://github.com/hugoocoto/runtime-arena/issues/56)
* **cli:** export public round materials for contestants ([#70](https://github.com/hugoocoto/runtime-arena/issues/70)) ([72d4196](https://github.com/hugoocoto/runtime-arena/commit/72d41966db1ba0219e8e2aeebeed4109984b9d32)), closes [#46](https://github.com/hugoocoto/runtime-arena/issues/46) [#52](https://github.com/hugoocoto/runtime-arena/issues/52) [#59](https://github.com/hugoocoto/runtime-arena/issues/59)
* **judge:** show queue load and submission timing ([#75](https://github.com/hugoocoto/runtime-arena/issues/75)) ([368be4b](https://github.com/hugoocoto/runtime-arena/commit/368be4baea5b04439aa95717d9a772a405bd82b7))
* **server:** audit refused submissions ([#72](https://github.com/hugoocoto/runtime-arena/issues/72)) ([54732e5](https://github.com/hugoocoto/runtime-arena/commit/54732e5881c7194ce24edc723b874b6c77102619))


### Bug Fixes

* **cli:** skip compiled binaries when archiving submission sources ([#69](https://github.com/hugoocoto/runtime-arena/issues/69)) ([25a6980](https://github.com/hugoocoto/runtime-arena/commit/25a69800b4f2ae39e0a92a94c6968a54c9eec526)), closes [#57](https://github.com/hugoocoto/runtime-arena/issues/57)
* **cli:** target the judge architecture for builds ([#73](https://github.com/hugoocoto/runtime-arena/issues/73)) ([a9ee808](https://github.com/hugoocoto/runtime-arena/commit/a9ee808fe7d5d1119cb9441be916f044eddccab5))
* **cli:** verify LAN CA fingerprints and support offline trust ([#74](https://github.com/hugoocoto/runtime-arena/issues/74)) ([306a22c](https://github.com/hugoocoto/runtime-arena/commit/306a22c11f81fb6ab2d3d6ceb46cbf3632761a4a)), closes [#43](https://github.com/hugoocoto/runtime-arena/issues/43) [#54](https://github.com/hugoocoto/runtime-arena/issues/54) [#61](https://github.com/hugoocoto/runtime-arena/issues/61) [#62](https://github.com/hugoocoto/runtime-arena/issues/62)
* **server:** keep problem materials available and preflight round starts ([#67](https://github.com/hugoocoto/runtime-arena/issues/67)) ([4601749](https://github.com/hugoocoto/runtime-arena/commit/4601749dd402d4c9874ac367f078206edbf7aa9f)), closes [#42](https://github.com/hugoocoto/runtime-arena/issues/42) [#45](https://github.com/hugoocoto/runtime-arena/issues/45) [#49](https://github.com/hugoocoto/runtime-arena/issues/49)
* **web:** honor problem deep links on the scoreboard ([#68](https://github.com/hugoocoto/runtime-arena/issues/68)) ([ae09911](https://github.com/hugoocoto/runtime-arena/commit/ae099112abc52b87b76d6d2639bb8b8f8e3a17a5)), closes [#60](https://github.com/hugoocoto/runtime-arena/issues/60)

## [0.11.1](https://github.com/hugoocoto/runtime-arena/compare/v0.11.0...v0.11.1) (2026-11-24)


### Bug Fixes

* **server:** harden limits, websockets and judging found in review ([#34](https://github.com/hugoocoto/runtime-arena/issues/34)) ([9c79b04](https://github.com/hugoocoto/runtime-arena/commit/9c79b0405b2b8677f8c3a56df0c89c505fdfe3b8))

## [0.11.0](https://github.com/hugoocoto/runtime-arena/compare/v0.10.0...v0.11.0) (2026-11-24)


### Features

* **cli:** add pull command to fetch the build image ([#29](https://github.com/hugoocoto/runtime-arena/issues/29)) ([fdac484](https://github.com/hugoocoto/runtime-arena/commit/fdac484fd9affe9b580fbbe67b00502b93fdef42))
* **cli:** add specs command showing the judge machine ([#32](https://github.com/hugoocoto/runtime-arena/issues/32)) ([47d27d0](https://github.com/hugoocoto/runtime-arena/commit/47d27d076923a37693fd82c9ca19cd08a08cd9b5))

## [0.10.0](https://github.com/hugoocoto/runtime-arena/compare/v0.9.1...v0.10.0) (2026-11-22)


### Features

* **web:** redesign the web UI and add a participant guide ([#26](https://github.com/hugoocoto/runtime-arena/issues/26)) ([c9f3962](https://github.com/hugoocoto/runtime-arena/commit/c9f3962779ee6cca3c503d746062618605c95a7c))

## [0.9.1](https://github.com/hugoocoto/runtime-arena/compare/v0.9.0...v0.9.1) (2026-11-22)


### Bug Fixes

* **deps:** bump url to 2.5.9 ([#25](https://github.com/hugoocoto/runtime-arena/issues/25)) ([fc8a1d1](https://github.com/hugoocoto/runtime-arena/commit/fc8a1d181da8cb9414c3f710ba1be542483ab809))
* **hosting:** contest hosting and signup flow ([#21](https://github.com/hugoocoto/runtime-arena/issues/21)) ([1672e85](https://github.com/hugoocoto/runtime-arena/commit/1672e85c9ca7e9c46a57de9d6ab0bb1424b4d9c5))
* serve HTTPS to clients connecting by LAN IP ([#24](https://github.com/hugoocoto/runtime-arena/issues/24)) ([6d587a6](https://github.com/hugoocoto/runtime-arena/commit/6d587a660c2e9b7df689b477006a9a12d7c70c4e))

## [0.9.0](https://github.com/hugoocoto/runtime-arena/compare/v0.8.0...v0.9.0) (2026-11-21)


### Features

* Add automatic HTTPS and LAN trust setup ([#19](https://github.com/hugoocoto/runtime-arena/issues/19)) ([4af7524](https://github.com/hugoocoto/runtime-arena/commit/4af7524bb387a79067d76608b97d37bfe7389638))

## [0.8.0](https://github.com/hugoocoto/runtime-arena/compare/v0.7.0...v0.8.0) (2026-11-21)


### Features

* **judge:** support parallel submissions and resource limits ([#17](https://github.com/hugoocoto/runtime-arena/issues/17)) ([ad3bd3d](https://github.com/hugoocoto/runtime-arena/commit/ad3bd3d95f8c14c29757b92d55c472e3e9060c1a))

## [0.7.0](https://github.com/hugoocoto/runtime-arena/compare/v0.6.0...v0.7.0) (2026-11-21)


### Features

* improve participant onboarding and host checks ([#15](https://github.com/hugoocoto/runtime-arena/issues/15)) ([5408a2e](https://github.com/hugoocoto/runtime-arena/commit/5408a2efd013a1c379f93d2ebe44a58c4c9f1799))

## [0.6.0](https://github.com/hugoocoto/runtime-arena/compare/v0.5.0...v0.6.0) (2026-11-21)


### ⚠ BREAKING CHANGES

* remove admin-created accounts, POST /api/admin/users, /api/admin/users/{name}/token and arena login <server> <token>. CLI failures now use exit codes 2-7.

### Features

* add CLI accounts, rate limits, completions, man pages and installers ([#13](https://github.com/hugoocoto/runtime-arena/issues/13)) ([8448683](https://github.com/hugoocoto/runtime-arena/commit/84486837109d6b90a2d8c5d791994d16c9dccd94))

## [0.5.0](https://github.com/hugoocoto/runtime-arena/compare/v0.4.0...v0.5.0) (2026-11-21)


### Features

* add CLI prompts and update notices, bind tokens to their server ([#11](https://github.com/hugoocoto/runtime-arena/issues/11)) ([1bd0261](https://github.com/hugoocoto/runtime-arena/commit/1bd02611c9ab3102b64aa5546a6e85824c512718))

## [0.4.0](https://github.com/hugoocoto/runtime-arena/compare/v0.3.0...v0.4.0) (2026-11-21)


### Features

* **cli:** default admin login to localhost and .env on the host ([#9](https://github.com/hugoocoto/runtime-arena/issues/9)) ([c390095](https://github.com/hugoocoto/runtime-arena/commit/c3900954610d531e18f3a741f1459824904e8e2f))

## [0.3.0](https://github.com/hugoocoto/runtime-arena/compare/v0.2.0...v0.3.0) (2026-11-21)


### Features

* web sign-up and password login for participants ([#7](https://github.com/hugoocoto/runtime-arena/issues/7)) ([43ca42c](https://github.com/hugoocoto/runtime-arena/commit/43ca42c4b3f8ff29d2f44fca1d4af75efe14021b))

## [0.2.0](https://github.com/hugoocoto/runtime-arena/compare/v0.1.0...v0.2.0) (2026-11-20)


### Features

* **judge:** judge executables against problem tests in isolate ([#2](https://github.com/hugoocoto/runtime-arena/issues/2)) ([c962479](https://github.com/hugoocoto/runtime-arena/commit/c962479365486b07bd3948cdf361e657d57d047e))
* server, CLI, live boards and projector page ([#5](https://github.com/hugoocoto/runtime-arena/issues/5)) ([cfa3531](https://github.com/hugoocoto/runtime-arena/commit/cfa35319096d1235b5b7db1a651ae906e9e39e7f))


### Bug Fixes

* **ci:** match release-please's tagged TOML values in Cargo.lock path ([#4](https://github.com/hugoocoto/runtime-arena/issues/4)) ([f06bc7f](https://github.com/hugoocoto/runtime-arena/commit/f06bc7fab1431d2b33958e534bd8678c82f3b2ee))
