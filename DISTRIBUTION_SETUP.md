# VisualLLM Distribution Setup Guide

This document describes the GitHub repository secrets and external accounts
needed to set up automated distribution for VisualLLM. The v0.6.0 release
policy is intentionally conservative: Linux x86_64 is the only published
distribution. Windows, macOS, Homebrew, AUR, and Flatpak are deferred.

## Current v0.6.0 release state

The release workflow publishes the Linux `.deb`/AppImage only. Windows and
macOS builds and OS-signing variables are reserved for a future release after
the corresponding certificate and final-byte verification process has been
tested:

- `VISUALLLM_WINDOWS_OS_SIGNING_ENABLED`
- `VISUALLLM_MACOS_OS_SIGNING_ENABLED`

Missing Windows/macOS certificate secrets are not a v0.6.0 release condition.
Tauri updater-signing secrets remain required for the Linux release because
update metadata must never advertise unverifiable bytes.

## Required GitHub Repository Secrets

### Code Signing Secrets

These secrets are not required for v0.6.0; they are future-release setup.

| Secret Name | Description | How to Obtain |
|-------------|-------------|---------------|
| `MACOS_CERTIFICATE` | Base64-encoded .p12 Developer ID Application certificate | Export from Keychain Access on macOS, then `base64 -i certificate.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | Password for the .p12 certificate | The password you set when exporting the certificate |
| `WINDOWS_CERTIFICATE` | Base64-encoded .pfx code signing certificate | Export from Windows Certificate Manager, then `base64 -i certificate.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password for the .pfx certificate | The password you set when exporting the certificate |

### Auto-Update Signing Secrets

Every release bundle is signed with the update key so installed apps can
verify the download before applying it. The app embeds the matching public
key (`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`); the release
workflow signs with the private key via these two secrets.

| Secret Name | Description | How to Obtain |
|-------------|-------------|---------------|
| `TAURI_SIGNING_PRIVATE_KEY` | Private half of the updater keypair (content of `~/.tauri/visualllm.key`) | `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/visualllm.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password protecting that private key | The password used at `npx tauri signer generate` |

Regenerate the pair if ever needed (and update `plugins.updater.pubkey` in
`tauri.conf.json` to match):

```sh
npx tauri signer generate -w ~/.tauri/visualllm.key
```

### Distribution Channel Secrets

| Secret Name | Description | How to Obtain |
|-------------|-------------|---------------|
| `HOMEBREW_TAP_TOKEN` | GitHub Personal Access Token with `repo` scope for pushing to homebrew-visualllm | Create at https://github.com/settings/tokens |
| `AUR_SSH_PRIVATE_KEY` | SSH private key for pushing to AUR | Generate with `ssh-keygen -t ed25519 -C "aur@visualllm"` |
| `AUR_SSH_PUBLIC_KEY` | SSH public key for AUR | The `.pub` file from above |
| `AUR_GPG_PRIVATE_KEY` | GPG private key for signing AUR packages | Export with `gpg --export-secret-keys --armor <key-id>` |
| `AUR_GPG_PASSPHRASE` | Passphrase for the GPG key | The passphrase you set for the GPG key |
| `FLATHUB_SUBMIT_TOKEN` | Flathub API token for submitting Flatpak builds | Generate at https://flathub.org/settings/tokens |

## Future and deferred distribution channels

Homebrew is deferred until a signed and notarized macOS DMG can back a proper
cask. Do not enable the current Linux tarball/formula updater.

AUR is deferred until the package account and manifest are ready. Flatpak is
deferred until its manifest, runtime, metadata, and publication path pass
independently. These channels are not part of the supported v0.6.0 release
journey.

### Future Homebrew Tap

1. Create a new repository: `CreativeSystemDesign/homebrew-visualllm`
2. Add the `HOMEBREW_TAP_TOKEN` secret to the main visualllm repo
3. The workflow will automatically update the formula on each release

### Future AUR (Arch User Repository)

1. Register an account at https://aur.archlinux.org/
2. Create a new package: `visualllm` (or adopt if it exists)
3. Add SSH key to your AUR account settings
4. Add GPG key for package signing
5. Add all AUR secrets to the main visualllm repo

### Future Flathub

1. Create a Flathub account at https://flathub.org/
2. Submit a new app request for `com.visualllm.VisualLLM`
3. Once approved, generate an API token at https://flathub.org/settings/tokens
4. Add `FLATHUB_SUBMIT_TOKEN` secret to the main visualllm repo

### Future macOS Code Signing

1. Join Apple Developer Program ($99/year)
2. Create a "Developer ID Application" certificate in Apple Developer portal
3. Download and install in Keychain Access
4. Export as .p12 with a password
5. Base64 encode: `base64 -i certificate.p12 | pbcopy`
6. Add `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD` secrets

### Future Windows Code Signing

1. Purchase a code signing certificate from a trusted CA (DigiCert, Sectigo, etc.)
2. Install the .pfx file on Windows
3. Export with private key if needed
4. Base64 encode: `base64 -i certificate.pfx | clip`
5. Add `WINDOWS_CERTIFICATE` and `WINDOWS_CERTIFICATE_PASSWORD` secrets

## Release Process

1. Verify the version and validation baseline locally.
2. Tag a release only after the user approves it: `git tag v0.6.0`.
3. GitHub Actions builds the Linux artifacts only. Windows/macOS remain
   deferred until their native build and signing process is intentionally
   resumed.
4. When OS signing is enabled, the required ordering is:
   build → OS sign/notarize → regenerate Tauri updater signatures over the
   final bytes → publish → verify hashes, signatures, and release labels.
5. Publish only the promoted artifact set. Deferred channels remain outside the
   core release result until they are independently verified.

Tauri updater signing is not Windows Authenticode or Apple code signing. Both
are required before a platform can be described as signed and trusted.

## Manual Verification

After a release, verify:
- [ ] GitHub Release has all artifacts
- [ ] Linux `.deb`/AppImage names and checksums match the release guidance
- [ ] Windows/macOS packages remain absent from v0.6.0
- [ ] Homebrew, AUR, and Flatpak remain absent until their deferred acceptance
      criteria pass

## Troubleshooting

### Homebrew tap not updating
- Check `HOMEBREW_TAP_TOKEN` has `repo` scope
- Verify the tap repository exists and is accessible

### AUR push fails
- Verify SSH key is added to AUR account
- Check GPG key is valid and not expired
- Ensure package name matches exactly

### Flatpak submission fails
- Verify Flathub token is valid
- Check app ID matches (`com.visualllm.VisualLLM`)
- Ensure manifest passes `flatpak-builder --lint`

### macOS signing fails
- Verify certificate is "Developer ID Application" type
- Check certificate hasn't expired
- Ensure keychain is unlocked in CI (handled by apple-actions/import-codesign-certs)

### Windows signing fails
- Verify certificate is valid for code signing
- Check timestamp server is accessible
- Ensure .pfx includes private key
