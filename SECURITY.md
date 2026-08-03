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

Provider keys are stored in the operating system keychain. The local
`providers.json` configuration contains blank key fields, and the renderer
receives only a masked hint (`has_key` plus a short non-secret marker). Existing
legacy files that contain plaintext keys are read for migration compatibility;
the next provider save rewrites the file without the key and imports the value
into the keychain when available.

The engine and the desktop UI are separate trust surfaces. The renderer has no
general filesystem or network capability; provider requests, keychain access,
state persistence, and the loopback listener are Rust-owned commands. The
clipboard permission is write-only and is used for copying endpoints and setup
instructions.

Users should not expose the engine to a LAN or the public internet without
adding authentication and reviewing the threat model. Any local process that
can access the loopback interface may call the engine; the local endpoint does
not authenticate clients because it is intended for same-user desktop tools.
Provider API keys are never included in lane responses, incident receipts,
preview fixtures, or normal error messages. Preview output can still contain
lane names, model metadata, and incident evidence, so it must be kept outside
the repository and shared only after review.
