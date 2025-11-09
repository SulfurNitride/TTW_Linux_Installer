# Bundled Binaries

This document tracks all bundled third-party binaries, their versions, and sources.

## xdelta3

- **Version:** 3.1.0
- **Source:** https://github.com/jmacd/xdelta
- **License:** Apache 2.0
- **Build Date:** Unknown (pre-built binary)
- **SHA1 (linux-x64):** b64031ee8450f148a52bc10ff82e46bdee245ea2
- **Purpose:** Binary patching for OpType 2 (Patch) operations

### Build Information

The linux-x64 binary is an ELF 64-bit LSB pie executable, dynamically linked.
Built for GNU/Linux 4.4.0.

### Verification

```bash
./TtwInstaller/BundledBinaries/linux-x64/xdelta3 -V
# Should output: Xdelta version 3.1.0, Copyright (C) Joshua MacDonald
```

### Notes

- Consider migrating to system-installed xdelta3 to avoid bundling
- If bundling is required, document the exact source/build process
- Track upstream releases: https://github.com/jmacd/xdelta/releases
