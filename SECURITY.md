# Security policy

## Supported versions

Only the latest release and the `main` branch receive security fixes.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting instead:

1. Go to the [Security tab](https://github.com/psymariux/Psytify/security) of this repository.
2. Choose **Report a vulnerability** and describe the problem.

Please include the affected version or commit, the platform, the steps to reproduce, and the impact
you expect. Do not include real tokens, backend addresses or personal data.

You can expect an acknowledgement within **5 days**. We will keep you updated while the issue is
investigated and fixed, and credit you in the advisory if you wish.

## Scope

In scope: the client in this repository (desktop and Android), its handling of the sign-in session, signed
links and local files, and the build and release workflows.

Out of scope: vulnerabilities in third-party backends you point the client at, and in upstream
dependencies (report those upstream; we track them with `cargo audit` and Dependabot).

## Design notes

- The client connects to a single configured backend and nothing else.
- Signed media links are treated as secrets: they are never written to logs or to disk.
- The optional sign-in uses OAuth 2.0 with PKCE; the refresh token is stored in a file readable only by
  the current user.
- Releases are built in GitHub Actions with pinned actions and carry build-provenance attestations
  (`gh attestation verify <file> --repo psymariux/Psytify`).
- Dependencies are checked on every pull request (`cargo deny`, RustSec audit, dependency review) and by
  CodeQL.
