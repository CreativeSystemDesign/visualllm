# Security Policy

## Supported versions

Security fixes target the latest version on the `main` branch and the latest
published release. Older versions may not receive fixes.

## Reporting a vulnerability

Please do not open a public issue for a suspected security vulnerability.

Until a dedicated security contact is published, contact the repository owner
through the private contact method associated with the GitHub account
`CreativeSystemDesign`, including:

- a concise description of the issue;
- affected files, versions, or configurations;
- reproduction steps that do not include real API keys;
- the possible impact.

Never include provider API keys, tokens, personal data, or full private
conversations in a report. Redact them before sending diagnostics.

## Security model

VisualLLM is a local desktop application. The default engine binds to
`127.0.0.1` and is not intended to be exposed to a network. Provider keys are
owned by the Rust side of the application and are not returned to the webview.

The current implementation stores provider keys in a local application-data
file protected by owner-only permissions on Unix systems. This is a known
limitation. OS keychain storage is planned before broad public adoption.

Users should not expose the engine to a LAN or the public internet without
adding authentication and reviewing the threat model.
