<img width="2325" height="1895" alt="image" src="https://github.com/user-attachments/assets/699715e9-47cc-4914-858c-d05427a57b8d" />

A linux native installer for MPI packages (TTW, BSA Decompressors, etc.).

To run it all you need to do is either double click it and allow executing, or `chmod +x mpi_installer_gui`.

## Where to get MPI files?

**TTW:** https://mod.pub/ttw/133/files - need to make an account and download the latest Tale of Two Wastelands file. Extract it and the mpi file will be inside.

**FNV BSA DECOMPRESSOR:** https://www.nexusmods.com/newvegas/mods/65854

**OBLIVION BSA DECOMPRESSOR:** https://www.nexusmods.com/oblivion/mods/49652

**Requirements:** None! (Everything is bundled)

## Performance Note

This installer uses parallel processing for speed - it will max out your CPU during audio processing (this is normal and good). TTW installs typically take 15-30 minutes instead of 1-2 hours.

## Building from Source

```bash
# Install dependencies (Arch/CachyOS)
sudo pacman -S rust xdelta3 alsa-lib

# Install dependencies (Ubuntu/Debian)
sudo apt install rustc cargo xdelta3 libasound2-dev

# Build
cargo build --release

# Binaries will be in target/release/
```

## CLI Usage

```bash
# Install TTW
./mpi_installer install --mpi /path/to/TTW.mpi --fo3 /path/to/Fallout3 --fnv /path/to/FalloutNV --dest /path/to/FalloutNV/Data

# Install Oblivion Decompressor
./mpi_installer install --mpi /path/to/OblDecomp.mpi --oblivion /path/to/Oblivion --dest /path/to/Oblivion/Data

# Other commands
./mpi_installer --help
```
