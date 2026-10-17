# CantiniAutoGPS

**Geotag your photos from a GPS track.** Drop in your photos and a GPX file from your phone, watch or GPS logger, and CantiniAutoGPS writes the location into each photo by matching the time it was taken to where you were at that moment.

It is a small desktop app for macOS, Windows and Linux, written in Rust. ExifTool is bundled inside it, so there is nothing else to install.

## Features

- **Works with your RAW files**: DNG, CR2, CR3, NEF, ARW, ORF, RAF, RW2, plus JPEG, HEIC and TIFF.
- **Drag and drop** photos, whole folders and GPX files, or use the Add buttons.
- **Several GPX files at once**, handy for multi-day trips.
- **Check before you tag**: the app tells you how many photos fall inside your track ("37 of 40 photos fall within the track") and shows both time spans, so a wrong time zone is obvious before anything is written.
- **Time zone detected automatically** from your photos, with a manual override.
- **Clock correction**: if your camera clock was off by a minute, enter the difference in seconds.
- **Safe by default**
  - Keeps an untouched backup of every photo (`name.ext_original`).
  - Leaves photos that already have a location alone, unless you ask it to replace them.
- **Clear results**: every photo shows whether it was tagged, skipped, or had no matching point in the track, with a summary at the end.
- **Progress and Cancel** while tagging.
- Follows your system's light or dark appearance and remembers your settings.

## Install

### macOS

**Requirements:** a Mac with Apple silicon (M1 or later). The app uses the Perl that ships with macOS.

1. Download `CantiniAutoGPS-<version>-macOS-arm64.dmg` from the [latest release](https://github.com/rmcantini/CantiniAutoGPS/releases/latest).
2. Open the DMG and drag **CantiniAutoGPS** into your **Applications** folder.
3. Open it for the first time using the steps below.

#### Opening an app that is not notarized

CantiniAutoGPS is free and open source and is **not signed with a paid Apple Developer ID or notarized by Apple**. macOS therefore blocks it the first time with a message such as *"CantiniAutoGPS can't be opened because Apple cannot check it for malicious software."* This is expected, and you only have to do this once.

**macOS 15 Sequoia and later**

1. Try to open CantiniAutoGPS from Applications. Click **Done** (not "Move to Bin") on the warning.
2. Open **System Settings → Privacy & Security** and scroll down to the **Security** section.
3. Next to *"CantiniAutoGPS" was blocked to protect your Mac*, click **Open Anyway**.
4. Confirm with your password or Touch ID, then click **Open**.

**macOS 14 Sonoma and earlier**

Right-click (or Control-click) the app, choose **Open**, then click **Open** in the dialog.

**Alternative, using Terminal**

This removes the "downloaded from the internet" flag from the app:

```bash
xattr -dr com.apple.quarantine /Applications/CantiniAutoGPS.app
```

### Windows

**Requirements:** Windows 10 or 11, 64-bit. (Windows builds are new in version 1.1.)

1. Download `CantiniAutoGPS-<version>-windows-x64.zip` from the [latest release](https://github.com/rmcantini/CantiniAutoGPS/releases/latest).
2. Right-click the zip and choose **Extract All…**. Run it from the extracted folder, not from inside the zip, and keep the folder together: `CantiniAutoGPS.exe` needs `exiftool.exe` and the `exiftool_files` folder next to it.
3. Double-click `CantiniAutoGPS.exe`.

The app is **not code-signed**, so Windows SmartScreen may show *"Windows protected your PC"*. Click **More info**, then **Run anyway**. You only need to do this once. If you prefer, right-click the downloaded zip, choose **Properties**, tick **Unblock**, and extract it afterwards.

### Linux

**Requirements:** 64-bit Intel/AMD (x86_64), a desktop with OpenGL, and a distribution with glibc 2.35 or newer (for example Ubuntu 22.04+, Debian 12+, Fedora 36+, Arch). Perl must be installed, which it is by default on virtually all desktop distributions. File dialogs use `xdg-desktop-portal`, which every major desktop provides.

1. Download `CantiniAutoGPS-<version>-linux-x86_64.AppImage` from the [latest release](https://github.com/rmcantini/CantiniAutoGPS/releases/latest).
2. Make it executable and run it:

```bash
chmod +x CantiniAutoGPS-*-linux-x86_64.AppImage
./CantiniAutoGPS-*-linux-x86_64.AppImage
```

AppImages need FUSE 2. If you see an error about `libfuse.so.2` (common on Ubuntu 22.04 and newer), either install it (`sudo apt install libfuse2`; on Ubuntu 24.04 the package is `libfuse2t64`) or run the app without FUSE:

```bash
./CantiniAutoGPS-*-linux-x86_64.AppImage --appimage-extract-and-run
```

The Linux build is automatically smoke-tested on a virtual display in CI; please [report any issues](https://github.com/rmcantini/CantiniAutoGPS/issues) from real desktops.

If you would rather not trust a binary, you can [build it yourself](#build-from-source).

## How to use it

1. **Add Photos…** or drag photos and folders onto the window.
2. **Add GPX…** or drag in your track file(s).
3. Check **Time and options**. The camera time zone is filled in from your photos when they contain it. Otherwise set the time zone your camera clock was using.
4. Look at the line under the options. If it says all photos fall within the track, you are good to go.
5. Click **Tag Photos**.

Open **Details** at the bottom for the full log.

### Tips

- **Time zone is the usual culprit.** Cameras store local time without a zone. If few photos match, the time zone is almost always the reason.
- **Camera clock off?** Use *Clock correction*. If the camera clock ran behind, enter a positive number of seconds; if it ran ahead, enter a negative one.
- **Track points far apart?** Locations are interpolated between track points, but only when the points are close enough in time. Otherwise the nearest point is used.
- **Re-running** on a photo that already has a backup keeps the first backup, so the `_original` file is always your untouched photo. Delete the `_original` files once you are happy with the result.
- Photos are modified in place. Keep *Keep a backup copy of each photo* ticked unless you have your own backups.

## How it works

For each photo, CantiniAutoGPS runs [ExifTool](https://exiftool.org) with your GPX track and the camera time zone, and ExifTool writes the matching GPS coordinates into the file. The app reads capture times first so it can show you the coverage check, and then tags photos one at a time so it can show progress and let you cancel.

## Build from source

You need [Rust](https://rustup.rs). From the project folder:

```bash
cargo run --release     # run the app
cargo test              # run the tests
```

To build the macOS app bundle and DMG, install [cargo-bundle](https://github.com/burtonageo/cargo-bundle) and run:

```bash
cargo install cargo-bundle
cargo bundle --release
```

The result is in `target/release/bundle/`. On macOS the bundled ExifTool lives in `vendor/exiftool/`. On Windows the app looks for `exiftool.exe` next to itself (download it from [exiftool.org](https://exiftool.org)). If no bundled copy is found, the app falls back to an ExifTool installed on your system (for example `brew install exiftool`).

GitHub Actions (`.github/workflows/build.yml`) builds and tests all three platforms on every push, and publishes the macOS DMG, Windows zip and Linux AppImage when a `v*` tag is pushed. Run `cantini_auto_gps --check-exiftool` to see which one is being used.

## License

CantiniAutoGPS is released under the [MIT License](LICENSE).

It bundles and uses other software under their own licenses:

- **[ExifTool](https://exiftool.org)** by Phil Harvey, bundled unmodified (in `vendor/exiftool/` on macOS and Linux; the official Windows build in the Windows zip), under the same terms as Perl (Artistic License or GPL). It is redistributed under the Artistic License; see [`vendor/exiftool/LICENSE-Artistic.txt`](vendor/exiftool/LICENSE-Artistic.txt).
- **Rust crates** (egui/eframe, rfd, image and their dependencies), all under permissive licenses such as MIT, Apache-2.0, BSD, Zlib and Unicode. The full list and license texts are in [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md), which is also included inside the app bundle.
- **Fonts**: the interface uses the system font (San Francisco on macOS, Segoe UI on Windows; egui's embedded font elsewhere) at runtime (not redistributed) and egui's embedded default fonts as a fallback (licenses in `THIRD_PARTY_LICENSES.md`).

## Acknowledgements

Thanks to Phil Harvey for ExifTool, which does the hard part, and to the egui project for the UI toolkit.
