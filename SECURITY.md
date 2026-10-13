# Security policy

CompareGraph reads source trees and writes a local SQLite store. It makes no network call and runs no code from the
repositories it indexes; an external provider plugin runs as a separate process with bounded reads and waits
(`docs/dev/plugin-protocol.md`). A report is in scope when CompareGraph can be made to execute code from an indexed
repository, read or write outside the workspace, hang or exhaust memory on a crafted input, or when a dependency it ships
has a known vulnerability.

## Reporting

Write to **support@massrelay.ai** or open a private security advisory on this repository (Security tab, "Report a
vulnerability"). Include the version (`comparegraph --version`), the platform, and a minimal input. You will get an
acknowledgement within seven days and a fix or a reasoned decision within ninety. Please do not open a public issue for
a vulnerability before a fix is released.

## Supported versions

The latest release on `main` and the latest tagged version. Older tags are not patched.
