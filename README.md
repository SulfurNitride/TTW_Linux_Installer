<img width="1860" height="1516" alt="image" src="https://github.com/user-attachments/assets/f3aa1651-b304-4def-acfb-495610765ba2" />


A linux native installer for MPI packages (TTW, BSA Decompressors, so far only tested TTW and FNV/Oblivion Decompressor).

To run it all you need to do is either double click it and allow executing, or `chmod +x mpi_installer_gui`.

## Where to get MPI files?

**TTW:** https://mod.pub/ttw/133/files - need to make an account and download the latest Tale of Two Wastelands file. Extract it and the mpi file will be inside.

**FNV BSA DECOMPRESSOR:** https://www.nexusmods.com/newvegas/mods/65854

**OBLIVION BSA DECOMPRESSOR:** https://www.nexusmods.com/oblivion/mods/49652

**Requirements:** None! (Everything is bundled)

## Building from Source

```bash
# Install dependencies (Arch)
sudo pacman -S rust xdelta3 alsa-lib

# Install dependencies (Debian)
sudo apt install rustc cargo xdelta3 libasound2-dev

# Build
cargo build --release

# Binaries will be in target/release/
```

## CLI Usage

```bash
# Install TTW
./mpi_installer install --mpi /path/to/TTW.mpi --fo3 /path/to/Fallout3 --fnv /path/to/FalloutNV --dest /path/to/output

# Install Oblivion Decompressor
./mpi_installer install --mpi /path/to/OblDecomp.mpi --oblivion /path/to/Oblivion --dest /path/to/output

# Other commands
./mpi_installer --help
```
