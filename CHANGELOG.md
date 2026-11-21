# Changelog

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
