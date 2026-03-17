# SimpleSync

A (vibe coded) simple file sync tool for Nextcloud and WebDAV servers, built with GTK4 and Libadwaita in Rust.

## Features

- Right now you can only select a local folder and upload/sync it to a remote destination

## Building

SimpleSync is built with Meson and Cargo. The easiest way to build is via GNOME Builder or Flatpak:

```
flatpak-builder --user --install --force-clean build io.github.nico359.simplesync.json
```

## Previous Version

The original Python implementation is preserved on the `python-legacy` branch.

## License

GPL-3.0-or-later

## AI Disclosure

This application was built with the assistance of AI (GitHub Copilot CLI, powered by Claude Haiku 4.5 and Claude Opus 4.6).
