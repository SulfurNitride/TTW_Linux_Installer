# TTW Installer - Quick Start

## What Is This?

A modern GUI installer for Tale of Two Wastelands 3.4, built with .NET and Avalonia UI.

## Ready to Use

**Location:** `TtwInstallerGui/publish/`

**Files needed to distribute (3 files, ~69MB):**
- `ttw_linux_gui` - Main executable
- `libbsa_capi.so` - BSA library
- `xdelta3` - Patch utility

## Running It

```bash
cd "TtwInstallerGui/publish"
./ttw_linux_gui
```

## User Requirements

The installer will check on startup and show if anything is missing:
- ✅ xdelta3 - **BUNDLED** (no install needed)
- ❌ ffmpeg - User must install: `sudo pacman -S ffmpeg`

## Key Features

1. **Auto MPI extraction** - Just point to the .mpi file
2. **Startup dependency check** - Shows what's missing
3. **Output directory validation** - Warns if not empty
4. **Parallel audio processing** - Uses multiple CPU cores
5. **No /tmp quota issues** - Staging uses output directory
6. **Error logging** - Saves detailed logs on failure
7. **Auto cleanup** - Removes temp files on shutdown

## Rebuilding

```bash
cd "TtwInstallerGui"
dotnet publish -c Release -r linux-x64
cp bin/Release/net9.0/linux-x64/publish/{ttw_linux_gui,libbsa_capi.so,xdelta3} publish/
```

## Recent Session Changes (Nov 7, 2024)

1. ✅ Bundled xdelta3 (generic x86-64 from Arch extra)
2. ✅ Fixed /tmp disk quota issue (now uses output directory)
3. ✅ Added startup dependency check popup
4. ✅ Added output directory validation dialog
5. ✅ Fixed Bazzite symlink disk space detection
6. ✅ Cleaned up all test directories and files

## Current Status

**Working:** GUI installer with all features implemented
**Tested:** Linux (Arch, Bazzite)
**Ready:** For distribution and testing

## Known Working Installations

Users have successfully installed on:
- Bazzite (with ffmpeg installed)
- Arch Linux systems

## If Something Breaks

Error logs saved to:
- `ttw-installer-error.log` (in current directory on failure)
- `<output>/ttw-installation.log` (detailed installation log)

## Full Documentation

See `PROJECT_STATUS.md` for complete technical documentation.
