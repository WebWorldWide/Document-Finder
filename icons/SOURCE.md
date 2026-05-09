# Icon Source Artwork

Master files for Document Finder branding. Not shipped at runtime.

- `Document Finder Icon.svg` — vector master (use for any new size).
- `Document Finder MacOS.png` — high-res raster master (1024×1024). Pass to `pnpm tauri icon` to regenerate every runtime size.
- `Document Finder Icon.png` — legacy raster master.
- `Document Finder.icon/` — macOS `.icon` bundle source.

To regenerate runtime icons after editing the master:

```bash
pnpm tauri icon "icons/Document Finder MacOS.png"
```

Tauri writes the result to `src-tauri/icons/`.
