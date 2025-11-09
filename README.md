<img width="2325" height="1895" alt="image" src="https://github.com/user-attachments/assets/699715e9-47cc-4914-858c-d05427a57b8d" />

This is a side project of mine. It's a linux native installer for MPI packages (TTW, BSA Decompressors, etc.). As of right now there might be issues, but in my testing it seems to work just fine. My testing was me loading it on The Badlands modlist with TTW.

**NEW:** Now works as a universal MPI installer! It auto-detects which game paths you need based on the MPI file you select.

To run it all you need to do is either double click it and allow executing. Or `chmod +x ttw_linux_gui`.

Not sure how much I'm going to update this I just wanted to share it.

Where to get MPI files?

**TTW:** https://mod.pub/ttw/133/files need to make an account, and and download the latest Tale of Two Wastelands file. Extract it and the mpi file will be inside.

**FNV BSA DECOMPRESSOR:** https://www.nexusmods.com/newvegas/mods/65854

**OBLIVION BSA DECOMPRESSOR:** https://www.nexusmods.com/oblivion/mods/49652

**Requirements:** FFMPEG I literally think this is the only thing you need.

## Performance Note

This installer uses parallel processing for speed - it will max out your CPU during audio processing (this is normal and good). TTW installs typically take 15-30 minutes instead of 1-2 hours.

## Building from Source

**Easy way (recommended):**
```bash
./build.sh
```

This creates a `release/` folder with everything you need, plus a `universal-mpi-installer-linux-x64.tar.gz` archive ready to share.

**Build script options:**
```bash
./build.sh --help        # Show all options
./build.sh --gui-only    # Build only GUI
./build.sh --cli-only    # Build only CLI
./build.sh --clean       # Clean build artifacts
```

**Manual way:**
```bash
# GUI
dotnet publish TtwInstallerGui/TtwInstallerGui.csproj -c Release -r linux-x64 --self-contained true -p:PublishSingleFile=true

# CLI
dotnet publish TtwInstaller/TtwInstaller.csproj -c Release -r linux-x64 --self-contained true
```
