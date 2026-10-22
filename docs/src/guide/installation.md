# Installation

Installing Quokka takes two steps:

1. Install [RGBDS](#prerequisite-rgbds), which Quokka uses to link ROMs.
2. Install the `quokka` compiler, either from a [pre-built release](#installing-a-pre-built-release) or by
   [building it from source](#building-from-source).

## Prerequisite: RGBDS

Quokka compiles your program into an object file and then hands it to [RGBDS](https://rgbds.gbdev.io/), the standard
Game Boy development toolchain, to finish the ROM:

- `rgblink` links the object file into a ROM.
- `rgbfix` fills in the ROM's header (the Nintendo logo and checksums), which a real Game Boy checks before running a
  game.

The [Hello, Quokka!](hello-quokka.md) tutorial also uses `rgbgfx` to convert images into Game Boy graphics.

All three tools come with RGBDS, and they must be on your `PATH`. Quokka is tested against RGBDS **v1.0.4**. Older
versions may not be able to read the object files Quokka produces.

The [RGBDS installation guide](https://rgbds.gbdev.io/install) covers every platform. Briefly:

- **macOS**: `brew install rgbds`
- **Linux**: some distributions package RGBDS, but the packaged version is often old. Check its version, and if it
  is older than v1.0.4, use a pre-built binary from the
  [RGBDS releases](https://github.com/gbdev/rgbds/releases) or build it from source as the installation guide
  describes.

Check that the tools are on your `PATH` and are recent enough:

```console
$ rgblink --version
rgblink v1.0.4
```

## Installing a pre-built release

Each [Quokka release](https://github.com/jhinch/quokka-lang/releases/latest) provides pre-built binaries for:

| Platform                       | Target                       |
|--------------------------------|------------------------------|
| macOS on Apple silicon         | `aarch64-apple-darwin`       |
| Linux on x86-64                | `x86_64-unknown-linux-musl`  |
| Linux on 64-bit ARM            | `aarch64-unknown-linux-musl` |

The Linux binaries are statically linked, so they work on any distribution. On any other platform (such as an Intel
Mac or Windows), [build Quokka from source](#building-from-source).

Go to the [latest release](https://github.com/jhinch/quokka-lang/releases/latest) and download the
`quokka-<version>-<target>.tar.gz` archive for your platform, along with its `.sha256` checksum file. For example,
for version `v0.3.0` on an Apple silicon Mac:

```sh
VERSION=v0.3.0
TARGET=aarch64-apple-darwin
curl -LO "https://github.com/jhinch/quokka-lang/releases/download/$VERSION/quokka-$VERSION-$TARGET.tar.gz"
curl -LO "https://github.com/jhinch/quokka-lang/releases/download/$VERSION/quokka-$VERSION-$TARGET.tar.gz.sha256"
```

Verify the download and extract it:

```sh
shasum -a 256 -c "quokka-$VERSION-$TARGET.tar.gz.sha256"
tar -xzf "quokka-$VERSION-$TARGET.tar.gz"
cd "quokka-$VERSION-$TARGET"
```

The extracted directory contains the `quokka` binary, along with the license and README.

### macOS: removing the quarantine attribute

The macOS binaries are not yet signed. If you downloaded the archive with a web browser, macOS quarantines it, and
Gatekeeper refuses to run `quokka` with a message saying it "cannot be opened because the developer cannot be
verified". Removing the quarantine attribute from the binary lets it run:

```sh
xattr -d com.apple.quarantine ./quokka
```

`curl` doesn't set the attribute, so if you downloaded the archive with the commands above, `xattr` reports that
there is no such attribute and you can skip this step.

### Putting `quokka` on your `PATH`

Move the binary into a directory on your `PATH`, for example:

```sh
mkdir -p ~/.local/bin
mv quokka ~/.local/bin/
```

If `~/.local/bin` isn't already on your `PATH`, add it in your shell's configuration file (such as `~/.zshrc` or
`~/.bashrc`):

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Then [check that the installation works](#checking-the-installation).

## Building from source

Building from source works on any platform with a Rust toolchain. Quokka uses the Rust 2024 edition, so you need
Rust 1.85 or newer. The easiest way to get it is through [rustup](https://rustup.rs/).

`cargo` can build and install the `quokka` binary straight from the repository:

```sh
cargo install --locked --git https://github.com/jhinch/quokka-lang quokka-lang
```

Or, to build from a local checkout (for example, if you plan to change the compiler):

```sh
git clone https://github.com/jhinch/quokka-lang.git
cd quokka-lang
cargo install --locked --path .
```

Either way, `cargo` installs `quokka` into `~/.cargo/bin`, which rustup adds to your `PATH`.

You don't need the repository's SameBoy submodule to build the compiler. Only Quokka's own emulator test suite uses
it.

## Checking the installation

Check that `quokka` runs:

```console
$ quokka --help
Usage: quokka [OPTIONS] <COMMAND>

Commands:
  build
  lsp
  help   Print this message or the help of the given subcommand(s)
...
```

With both Quokka and RGBDS installed, you're ready to build your first ROM in [Hello, Quokka!](hello-quokka.md).
