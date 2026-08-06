# VisualLLM Distribution Setup Guide

This document describes the GitHub repository secrets and external accounts needed to set up automated distribution for VisualLLM.

## Required GitHub Repository Secrets

### Code Signing Secrets

| Secret Name | Description | How to Obtain |
|-------------|-------------|---------------|
| `MACOS_CERTIFICATE` | Base64-encoded .p12 Developer ID Application certificate | Export from Keychain Access on macOS, then `base64 -i certificate.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | Password for the .p12 certificate | The password you set when exporting the certificate |
| `WINDOWS_CERTIFICATE` | Base64-encoded .pfx code signing certificate | Export from Windows Certificate Manager, then `base64 -i certificate.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password for the .pfx certificate | The password you set when exporting the certificate |

### Distribution Channel Secrets

| Secret Name | Description | How to Obtain |
|-------------|-------------|---------------|
| `HOMEBREW_TAP_TOKEN` | GitHub Personal Access Token with `repo` scope for pushing to homebrew-visualllm | Create at https://github.com/settings/tokens |
| `AUR_SSH_PRIVATE_KEY` | SSH private key for pushing to AUR | Generate with `ssh-keygen -t ed25519 -C "aur@visualllm"` |
| `AUR_SSH_PUBLIC_KEY` | SSH public key for AUR | The `.pub` file from above |
| `AUR_GPG_PRIVATE_KEY` | GPG private key for signing AUR packages | Export with `gpg --export-secret-keys --armor <key-id>` |
| `AUR_GPG_PASSPHRASE` | Passphrase for the GPG key | The passphrase you set for the GPG key |
| `FLATHUB_SUBMIT_TOKEN` | Flathub API token for submitting Flatpak builds | Generate at https://flathub.org/settings/tokens |

## Setting Up Each Distribution Channel

### 1. Homebrew Tap

1. Create a new repository: `CreativeSystemDesign/homebrew-visualllm`
2. Add the `HOMEBREW_TAP_TOKEN` secret to the main visualllm repo
3. The workflow will automatically update the formula on each release

### 2. AUR (Arch User Repository)

1. Register an account at https://aur.archlinux.org/
2. Create a new package: `visualllm` (or adopt if it exists)
3. Add SSH key to your AUR account settings
4. Add GPG key for package signing
5. Add all AUR secrets to the main visualllm repo

### 3. Flathub

1. Create a Flathub account at https://flathub.org/
2. Submit a new app request for `com.visualllm.VisualLLM`
3. Once approved, generate an API token at https://flathub.org/settings/tokens
4. Add `FLATHUB_SUBMIT_TOKEN` secret to the main visualllm repo

### 4. macOS Code Signing

1. Join Apple Developer Program ($99/year)
2. Create a "Developer ID Application" certificate in Apple Developer portal
3. Download and install in Keychain Access
4. Export as .p12 with a password
5. Base64 encode: `base64 -i certificate.p12 | pbcopy`
6. Add `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD` secrets

### 5. Windows Code Signing

1. Purchase a code signing certificate from a trusted CA (DigiCert, Sectigo, etc.)
2. Install the .pfx file on Windows
3. Export with private key if needed
4. Base64 encode: `base64 -i certificate.pfx | clip`
5. Add `WINDOWS_CERTIFICATE` and `WINDOWS_CERTIFICATE_PASSWORD` secrets

## Release Process

1. Tag a release: `git tag v0.1.0 && git push origin v0.1.0`
2. GitHub Actions will:
   - Build for Linux, Windows, macOS
   - Sign macOS and Windows artifacts
   - Create GitHub Release with all artifacts
   - Update Homebrew tap formula
   - Push PKGBUILD to AUR
   - Build and submit Flatpak to Flathub

## Manual Verification

After a release, verify:
- [ ] GitHub Release has all artifacts
- [ ] Homebrew formula updated at https://github.com/CreativeSystemDesign/homebrew-visualllm
- [ ] AUR package updated at https://aur.archlinux.org/packages/visualllm
- [ ] Flatpak available at https://flathub.org/apps/com.visualllm.VisualLLM
- [ ] macOS DMG is notarized (check with `spctl -a -v VisualLLM.dmg`)
- [ ] Windows MSI/EXE are signed (check with right-click → Properties → Digital Signatures)

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