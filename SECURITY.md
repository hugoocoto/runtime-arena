# Security

## Threat model

Runtime Arena executes **untrusted binaries** uploaded by participants. They run
inside [isolate](https://github.com/ioi/isolate) sandboxes: separate namespaces,
no network, a private filesystem view, cgroup CPU and memory limits, process
limits and a syscall filter.

To create those sandboxes, the server container runs with `privileged: true`
and its own cgroup namespace. isolate's authors do not recommend running it in
containers; we do it for easy deployment, and CI checks the sandbox verdicts
(time limit, memory limit, crash, no network) on every change. Consequences:

- A sandbox escape into the container is effectively root on the host.
- Host the judge on a machine dedicated to the contest, not one holding other
  data or services.
- Keep the image updated; isolate is pinned in `docker/Dockerfile`.

## Reporting a vulnerability

Please report privately through
[GitHub security advisories](https://github.com/hugoocoto/runtime-arena/security/advisories/new),
not public issues. Sandbox escapes found in isolate itself should also go to its
maintainers.
