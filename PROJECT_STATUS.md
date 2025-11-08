# TTW Installer - Project Status & Documentation

**Last Updated:** November 7, 2024

## Overview

A modern, cross-platform installer for Tale of Two Wastelands (TTW) built with .NET 9 and Avalonia UI. Provides both GUI and CLI interfaces for installing TTW 3.4 on Linux (with future Windows/macOS support).

## Features Implemented

### ✅ Core Functionality
- **Automatic MPI Extraction** - Detects and extracts .mpi files to temp directories
- **BSA Archive Support** - Full read/write support for Bethesda BSA archives
- **Parallel Audio Processing** - Uses CPU_CORES - 2 threads for OggEnc2/AudioEnc operations
- **xdelta3 Patch Application** - Bundled xdelta3 binary (no user installation required)
- **Comprehensive Validation** - Pre-flight checks for game files, disk space, and dependencies
- **Detailed Logging** - Installation logs saved to `ttw-installation.log` with error tracking
- **Temp File Management** - Automatic cleanup on shutdown and startup

### ✅ GUI Features
- **Path Configuration** - Browse buttons for all required paths (Fallout 3, New Vegas, MPI, Output)
- **Real-time Progress** - Progress bar with percentage and status text
- **Live Log Output** - Console-style log viewer with monospace font
- **Config Persistence** - Settings saved to `ttw-config.json`
- **Startup Dependency Check** - Popup dialog showing system requirements status
- **Output Directory Validation** - Warns if output directory is not empty
- **Error Logging** - Failed installations write `ttw-installer-error.log`

### ✅ Recent Improvements (This Session)
1. **Bundled xdelta3** - No longer requires system installation
   - Generic x86-64 binary from Arch extra repo (avoids AVX-512 issues)
   - Located next to executable (flat structure like libbsa_capi.so)
2. **Fixed /tmp Disk Quota Issue** - BSA staging now uses output directory instead of /tmp
3. **Startup Requirements Dialog** - Shows dependency status on launch
4. **Output Directory Check** - Warns before installing to non-empty directories
5. **Improved Linux Mount Detection** - Proper disk space checks for symlinked directories (e.g., Bazzite /home -> /var/home)

## Project Structure

```
TTW Installer/
├── BsaLib/                          # BSA library wrapper (C# bindings for libbsa)
├── TtwInstaller/                    # CLI installer project
│   ├── Models/                      # Data models (InstallConfig, Asset, Location, etc.)
│   ├── Services/                    # Core services
│   │   ├── AssetProcessor.cs        # Processes individual assets
│   │   ├── BsaReader.cs             # Reads BSA archives
│   │   ├── BsaWriter.cs             # Creates BSA archives
│   │   ├── BundledBinaryManager.cs  # Manages bundled xdelta3
│   │   ├── DependencyChecker.cs     # Checks system dependencies
│   │   ├── InstallationLogger.cs    # Tracks errors/warnings
│   │   ├── LocationResolver.cs      # Resolves asset paths
│   │   ├── ManifestLoader.cs        # Loads MPI index.json
│   │   ├── MpiExtractor.cs          # Extracts .mpi files
│   │   ├── PostCommandRunner.cs     # Runs post-install commands
│   │   ├── TempDirectoryTracker.cs  # Tracks temp directories for cleanup
│   │   └── ValidationService.cs     # Pre-flight validation
│   ├── Utils/                       # Utility classes
│   └── libbsa_capi.so              # BSA library native binary
├── TtwInstallerGui/                 # Avalonia GUI project
│   ├── ViewModels/                  # MVVM ViewModels
│   │   └── MainWindowViewModel.cs   # Main window logic
│   ├── Views/                       # XAML views
│   │   └── MainWindow.axaml         # Main window UI
│   ├── App.axaml.cs                 # App initialization + temp cleanup
│   ├── xdelta3                      # Bundled xdelta3 binary (163KB)
│   └── publish/                     # Distribution files
│       ├── ttw_linux_gui            # Main executable (68MB)
│       ├── libbsa_capi.so           # BSA library (486KB)
│       ├── xdelta3                  # Bundled binary (163KB)
│       └── README.md                # User documentation
└── PROJECT_STATUS.md                # This file
```

## Distribution Package

The final distributable is in `TtwInstallerGui/publish/`:

**Essential Files (3 files, ~69MB total):**
- `ttw_linux_gui` - Main executable (single-file, self-contained)
- `libbsa_capi.so` - Native BSA library
- `xdelta3` - Bundled patch binary

**Optional:**
- `README.md` - User documentation

## Dependencies

### Bundled (No Installation Required)
- ✅ xdelta3 - Bundled in distribution package

### System Dependencies (User Must Install)
- ❌ ffmpeg - Required for audio conversion

**Installation Instructions (Auto-detected by installer):**
- Arch/Manjaro: `sudo pacman -S ffmpeg`
- Ubuntu/Debian: `sudo apt install ffmpeg`
- Fedora: `sudo dnf install ffmpeg`
- Bazzite: `rpm-ostree install ffmpeg` (requires reboot)

## Building the Project

### Prerequisites
- .NET 9 SDK
- Linux (tested on Arch-based systems)

### Build Commands

**Development Build:**
```bash
cd "TtwInstallerGui"
dotnet build
```

**Release Build (GUI):**
```bash
cd "TtwInstallerGui"
dotnet publish -c Release -r linux-x64
```

Output: `bin/Release/net9.0/linux-x64/publish/`

**CLI Build:**
```bash
cd "TtwInstaller"
dotnet publish -c Release -r linux-x64
```

### Copy to Distribution Folder
```bash
cd "TtwInstallerGui"
cp bin/Release/net9.0/linux-x64/publish/ttw_linux_gui publish/
cp bin/Release/net9.0/linux-x64/publish/libbsa_capi.so publish/
cp bin/Release/net9.0/linux-x64/publish/xdelta3 publish/
```

## Running the Installer

### GUI
```bash
cd "TtwInstallerGui/publish"
./ttw_linux_gui
```

### CLI
```bash
cd "TtwInstaller"
dotnet run
```

Or with config file:
```bash
./TtwInstaller --config ttw-config.json
```

## Configuration File Format

`ttw-config.json`:
```json
{
  "Fallout3Root": "/path/to/fallout3",
  "FalloutNVRoot": "/path/to/falloutnv",
  "MpiPackagePath": "/path/to/Tale of Two Wastelands 3.4.mpi",
  "DestinationPath": "/path/to/output",
  "StartInstallation": true
}
```

## Known Issues & Solutions

### Issue: "Disk quota exceeded" in /tmp
**Fixed** - BSA staging now uses output directory instead of `/tmp`
- Location: `<output_directory>/.ttw_bsa_staging_temp/`

### Issue: Disk space showing 0 GB on Bazzite
**Fixed** - Improved mount point detection for symlinked directories
- Now properly resolves `/home` -> `/var/home` symlinks

### Issue: xdelta3 not found
**Fixed** - xdelta3 now bundled with installer
- Generic x86-64 binary from Arch extra repo
- Located next to executable

### Issue: Thousands of xdelta3 errors
**Fixed** - Pre-installation dependency check
- Startup dialog shows dependency status
- Clear installation instructions for missing dependencies

## Temp File Management

### MPI Extraction
- Location: `/tmp/ttw_mpi_<guid>/`
- Cleanup: Automatic on app shutdown + startup stale cleanup
- Tracked by: `TempDirectoryTracker.cs`

### BSA Staging
- Location: `<output_directory>/.ttw_bsa_staging_temp/`
- Cleanup: Automatic when `BsaWriter` is disposed
- Size: Can be several GB during installation

## Installation Flow

1. **Startup**
   - Show dependency check dialog
   - Load saved config from `ttw-config.json`
   - Clean stale temp directories

2. **Pre-Installation**
   - Validate all paths are specified
   - Check if output directory is empty (warn if not)
   - Check system dependencies (ffmpeg)
   - Verify bundled xdelta3

3. **Configuration Validation**
   - Run manifest validation checks
   - Verify game files exist
   - Check disk space

4. **MPI Extraction** (if .mpi file)
   - Extract to temp directory
   - Update config to use extracted path

5. **Asset Processing** (Progress: 10-80%)
   - New assets (10-20%)
   - Copy operations (20-50%)
   - Patches (50-60%)
   - OggEnc2 audio (60-75%) - **PARALLEL**
   - AudioEnc audio (75-80%) - **PARALLEL**

6. **BSA Writing** (80-95%)
   - Stage files to `.ttw_bsa_staging_temp/`
   - Pack BSA archives
   - Write to output directory
   - Clean up staging

7. **Post-Installation** (95-100%)
   - Run post-install commands
   - Write `ttw-installation.log`
   - Display summary

8. **Cleanup**
   - Delete MPI temp directory
   - Delete BSA staging directory
   - Close installer

## Error Handling

### Installation Failures
- Error log written to `ttw-installer-error.log` in current directory
- Contains full stack trace and installation log
- User can share for debugging

### Validation Failures
- Detailed error messages shown in GUI
- Specific file/check that failed
- Platform-specific fix instructions

### Missing Dependencies
- Startup dialog shows status
- Install instructions for user's OS
- Installation prevented if dependencies missing

## Performance Optimizations

1. **Parallel Audio Processing**
   - Uses `Environment.ProcessorCount - 2` threads
   - Applies to OggEnc2 and AudioEnc operations
   - Significantly faster on multi-core systems

2. **BSA Staging in Output Directory**
   - Avoids /tmp space limitations
   - Same disk = no cross-filesystem operations
   - Automatic cleanup

3. **Single-File Deployment**
   - 68MB compressed executable
   - All .NET runtime included
   - Fast startup

## Platform Support

### Currently Supported
- ✅ Linux x64 (tested on Arch, Bazzite)

### Planned
- ⏳ Windows x64
- ⏳ macOS (ARM64 + x64)

Platform-specific binaries needed:
- xdelta3 (Windows .exe, macOS binary)
- libbsa_capi (Windows .dll, macOS .dylib)

## Future Improvements

1. **Windows/macOS Support**
   - Bundle platform-specific binaries
   - Update paths to use cross-platform separators

2. **Bundle ffmpeg**
   - Eliminate last system dependency
   - Larger distribution (~100MB instead of ~69MB)

3. **Resume Capability**
   - Save progress state
   - Resume from last checkpoint

4. **Verification Mode**
   - Verify existing installation
   - Check file hashes

5. **Update Mechanism**
   - Check for TTW updates
   - Incremental updates

## Technical Notes

### BSA Archive Handling
- Uses custom libbsa C bindings
- Supports FNV/FO3 BSA formats
- Handles compression and embedded files

### xdelta3 Patching
- Applied to modified game assets
- Smaller download size than full files
- Bundled generic x86-64 binary

### Audio Conversion
- OggEnc2: Legacy audio format conversion
- AudioEnc: Modern audio conversion (uses ffmpeg)
- Both use parallel processing

### Archive Flags
- Properly sets BSA archive flags from manifest
- Supports meshes, textures, sounds, voices, shaders, trees, fonts

## Contact & Support

For issues or questions:
- GitHub Issues: (To be added)
- TTW Discord: (Link from TTW team)

## License

(To be determined - likely matches TTW project license)

---

**Project Status:** Ready for testing and distribution
**Version:** 1.0 (Pre-release)
**Build Date:** November 7, 2024
