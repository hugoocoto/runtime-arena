# CI only: the images the "sandbox smoke and end-to-end" job tests
# (.github/workflows/ci.yml). One bake call builds both targets in parallel.
# The gha caches only work inside GitHub Actions, so this is not for local use:
# scripts/sandbox-smoke.sh and scripts/e2e.sh build for themselves without SKIP_BUILD.
#
# Paths are relative to the working directory, which is the repository root.

# Set by GitHub Actions. Layer caches are saved on pushes to main only; PRs read them.
variable "GITHUB_EVENT_NAME" {
  default = ""
}

group "default" {
  targets = ["sandbox-programs", "server"]
}

# The programs are exported as files, so build-env is never loaded.
# release-image also reads the build-env scope.
target "sandbox-programs" {
  context    = "."
  dockerfile = "docker/Dockerfile"
  target     = "sandbox-programs"
  output     = ["type=local,dest=tests/sandbox/out"]
  cache-from = ["type=gha,scope=build-env"]
  cache-to   = GITHUB_EVENT_NAME == "push" ? ["type=gha,mode=max,scope=build-env"] : []
}

target "server" {
  context    = "."
  dockerfile = "docker/Dockerfile"
  target     = "server"
  # Must match the arena image in compose.yaml, or sandbox-smoke.sh pulls the
  # published image and tests that instead.
  tags   = ["ghcr.io/hugoocoto/runtime-arena-server:latest"]
  output = ["type=docker"]
  # Compile fast: these tests don't depend on the server being optimised.
  args = {
    CARGO_PROFILE_RELEASE_LTO           = "off"
    CARGO_PROFILE_RELEASE_OPT_LEVEL     = "0"
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "256"
  }
  # Its own cache scope keeps these layers apart from the release build.
  cache-from = ["type=gha,scope=server-ci"]
  cache-to   = GITHUB_EVENT_NAME == "push" ? ["type=gha,mode=max,scope=server-ci"] : []
}
