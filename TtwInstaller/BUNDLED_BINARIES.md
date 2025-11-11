# Bundled Binaries

This document tracks all bundled third-party binaries, their versions, and sources.

## xdelta3

- **Version:** 3.1.0
- **Source:** https://github.com/jmacd/xdelta-gpl
- **License:** Apache 2.0
- **Purpose:** Binary patching for OpType 2 (Patch) operations

### Platform Binaries

#### Linux (x86_64)
- **Location:** `TtwInstaller/BundledBinaries/linux-x64/xdelta3`
- **SHA1:** b64031ee8450f148a52bc10ff82e46bdee245ea2
- **Build Info:** ELF 64-bit LSB pie executable, dynamically linked, built for GNU/Linux 4.4.0
- **Download:** https://github.com/jmacd/xdelta-gpl/releases/download/v3.1.0/xdelta3-3.1.0-x86_64.exe.zip

### Verification

```bash
./TtwInstaller/BundledBinaries/linux-x64/xdelta3 -V
```

Should output: `Xdelta version 3.1.0, Copyright (C) Joshua MacDonald`

### Notes

- xdelta3 is bundled to ensure consistent patching behavior
- Track upstream releases: https://github.com/jmacd/xdelta-gpl/releases

## libbsa

- **Version:** 3.x (latest from GitHub)
- **Source:** https://github.com/Ortham/libbsa
- **License:** GNU General Public License v3.0
- **Purpose:** BSA archive extraction and manipulation for OpType 1 (ExtractBSA) operations

### Platform Binaries

#### Linux (x86_64)
- **Location:** `TtwInstaller/libbsa_capi.so` (in root TtwInstaller directory)
- **Build Info:** Built using system Boost libraries on Linux
- **Dependencies:** Boost (filesystem, locale, iostreams), zlib (dynamically linked)

### Building from Source

```bash
# Install dependencies
sudo apt-get install cmake build-essential libboost-all-dev zlib1g-dev

# Clone and build
git clone https://github.com/Ortham/libbsa.git
cd libbsa
mkdir build && cd build
cmake .. -DBUILD_SHARED_LIBS=ON
cmake --build . --config Release

# Output: build/libbsa.so (rename to libbsa_capi.so)
```

### Notes

- Uses system-installed Boost libraries (dynamically linked)
- Uses the C API from libbsa for BSA operations

## ffmpeg

- **Version:** 7.0.2
- **Source:** https://johnvansickle.com/ffmpeg/
- **License:** GPL (General Public License)
- **Purpose:** Audio processing for OpType 4 (ProcessOggEnc2) and OpType 5 (ProcessAudioEnc) operations

### Platform Binaries

#### Linux (x86_64)
- **Location:** `TtwInstaller/BundledBinaries/linux-x64/ffmpeg`
- **Version:** 7.0.2
- **Size:** ~77MB (static build)
- **Build Info:** Static build with all codecs included
- **Download:** https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz

### Fallback Behavior

The installer will automatically:
1. Check for bundled ffmpeg first (in the application directory)
2. Fall back to system PATH if bundled version not found
3. Show appropriate error message if neither is available

### Verification

```bash
./TtwInstaller/BundledBinaries/linux-x64/ffmpeg -version
```

Should output version information and list of enabled codecs.

### Notes

- ffmpeg is now bundled to provide zero-dependency installation
- Static build with no external dependencies
- Users can still use system ffmpeg if preferred (installer will detect and use it)
- Large file size is due to including all codecs and formats
