# TTW Linux Installer

A native Linux installer for Tale of Two Wastelands (TTW) and other MPI packages like the Oblivion Decompressor.

## Features

- Native Linux support (no Wine required for the installer itself)
- GUI and CLI interfaces
- BSA archive creation (v104/v105 format)
- xdelta3 binary patching
- Audio transcoding (MP3/WAV to OGG Vorbis)
- Multi-threaded asset processing

## Installation

### From Releases

Download the latest release from the [Releases](https://github.com/SulfurNitride/TTW_Linux_Installer/releases) page.

```bash
tar -xzf ttw-installer-linux-x86_64.tar.gz
cd release
./mpi_installer_gui  # GUI mode
# or
./mpi_installer --help  # CLI mode
```

### Building from Source

Requirements:
- Rust 1.70+
- xdelta3
- ALSA development libraries

```bash
# Install dependencies (Arch/CachyOS)
sudo pacman -S rust xdelta3 alsa-lib

# Install dependencies (Ubuntu/Debian)
sudo apt install rustc cargo xdelta3 libasound2-dev

# Build
cargo build --release

# Binaries will be in target/release/
```

## Usage

### GUI

```bash
./mpi_installer_gui
```

### CLI

**Install TTW:**
```bash
./mpi_installer install \
  --mpi /path/to/TTW_Main.mpi \
  --fo3 /path/to/Fallout3 \
  --fnv /path/to/FalloutNV \
  --dest /path/to/FalloutNV/Data
```

**Install Oblivion Decompressor:**
```bash
./mpi_installer install \
  --mpi /path/to/OblDecomp.mpi \
  --oblivion /path/to/Oblivion \
  --dest /path/to/Oblivion/Data
```

**Other commands:**
```bash
./mpi_installer inspect --mpi /path/to/package.mpi  # Inspect package contents
./mpi_installer extract --mpi /path/to/package.mpi --output /path/to/output  # Extract MPI
./mpi_installer verify --fo3 /path/to/Fallout3  # Verify game installation
./mpi_installer logs  # View recent installation logs
```

## Supported MPI Packages

- Tale of Two Wastelands (TTW) 3.x
- Oblivion Decompressor
- Other MPI-format mod packages

## Notes

- Game installations should be set up via Steam/GOG with Proton/Wine first
- The installer processes MPI packages natively on Linux
- BSA archives are created in the correct format for each game
- Audio files are automatically transcoded to OGG Vorbis format

## License

MIT
