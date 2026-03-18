# SimpleSync

A (vibe coded) simple file sync tool for Nextcloud and WebDAV servers, built with GTK4 and Libadwaita in Rust. 

My Motivation for this was the fact that UBSync is specifically made for Ubuntu Touch and the Nextcloud Desktop Client is not really made for using it on a mobile device. Also it has no option to just push local changes without syncing all the remote content (at least that I know of). This is especially annoying if you want to upload e.g. pictures of your device to a folder that already contains a lot of files because it will try to download everything from the server which is not desirable behaviour in my opinion. I wanted something similat to the auto upload feature of the Nextcloud Android/iOS app without having to mess around with rsync or something similar. Therefore I deciedd to create this simple app.

## Features

- Push local content to remote with the press of a button
- Pull remote content to a local folder also with the press of a button
- Configure as many targets as you want and optionally push them all at once

## Building

SimpleSync is built with Meson and Cargo. The easiest way to build is via GNOME Builder IDE or flatpak-builder:

```
flatpak-builder --user --install --force-clean build io.github.nico359.simplesync.json
```

## Previous Version

The original Python implementation is preserved on the `python-legacy` branch.

## License

GPL-3.0-or-later

## AI Disclosure

This application was built with the assistance of AI (GitHub Copilot CLI, Claude Opus 4.6).
