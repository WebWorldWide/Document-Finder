# Releasing Document Finder

## Cut a release

```bash
# Bump version in src-tauri/Cargo.toml AND package.json
git add src-tauri/Cargo.toml package.json
git commit -m "chore: bump version to x.y.z"
git tag vx.y.z
git push origin main --tags
```

The `release.yml` workflow builds unsigned installers for macOS (universal dmg),
Linux (AppImage + deb), and Windows (msi + exe) and attaches them to a **draft**
GitHub Release. Review the draft, edit the notes, then publish.

## Install unsigned macOS build

1. Download `Document.Finder_*_universal.dmg` from the GitHub release page.
2. Open the DMG and drag **Document Finder** to `/Applications`.
3. On first launch Gatekeeper will block it.
4. Open **System Settings → Privacy & Security**, scroll down to the blocked-app
   notice, click **Open Anyway**, then confirm.

Alternatively, from Terminal:
```bash
xattr -dr com.apple.quarantine /Applications/Document\ Finder.app
```

## Verify the binary

```bash
codesign -dvvv /Applications/Document\ Finder.app
# Expect "not signed" — that's expected for an unsigned build.

# Check for unexpected outbound entitlements:
codesign -d --entitlements - /Applications/Document\ Finder.app
```

## CI workflow

`ci.yml` runs on every push to `main` and on pull requests:
- **frontend**: type-check + Vite build (ubuntu-latest)
- **rust**: `cargo check` (macos-14, matches release runner)
