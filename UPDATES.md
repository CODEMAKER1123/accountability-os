# Application updates

Accountability OS checks GitHub Releases shortly after startup and every four hours while it remains running. When a newer signed version is available, the main sidebar shows an **Update ready** card with a blue download button. Clicking it downloads and verifies the update, installs it, and restarts the app.

Updates are never installed silently. The update manifest and Windows updater bundle must both have a valid Tauri updater signature.

## Publishing an update

1. Bump the version in `package.json`, the root `Cargo.toml`, and `src-tauri/tauri.conf.json`.
2. Merge the tested change to `main`.
3. Create and push the matching tag, such as `v0.2.0`.
4. Approve the protected `release` environment deployment in GitHub Actions.
5. The `Publish signed updater release` workflow builds the signed NSIS updater, creates the GitHub Release, and uploads `latest.json`.

The workflow rejects tags that do not exactly match the configured application version.

## Building a signed installer locally

On the Windows machine holding the protected key and Credential Manager entry, run:

```powershell
npm run build:signed
```

The helper reads the encrypted private key and its password only into the build process, then clears both environment variables. It does not print either secret. Other contributors should use the GitHub release workflow rather than sharing the signing key.

## Signing-key custody

- Protected GitHub Actions `release` environment secrets: `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- The environment accepts only `v*` tags and requires approval from the repository owner.
- Protected local backup: `%USERPROFILE%\.tauri\accountability-os-updater.key`.
- Local password target: Windows Credential Manager entry `tauri-updater.accountability-os`.
- Only the public key is committed in `src-tauri/tauri.conf.json`.

Back up the encrypted private key outside this computer. Losing it prevents installed copies from trusting future updates.
