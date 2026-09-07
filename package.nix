{
  lib,
  rustPlatform,
  pkg-config,
  dbus,
  libGL,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxkbcommon,
  libxrender,
  wayland,
  zlib,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "ttw-linux-installer";
  version = "0.2.0";

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./examples
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [pkg-config];

  # libz-sys links against zlib, x11-dl bakes the libdir pkg-config reports for the X11 libraries into the binary.
  buildInputs = [
    zlib
    libGL
    libx11
    libxcursor
    libxi
    libxrender
  ];

  # Reached by bare soname instead: glutin, xkbcommon-dl, wayland-sys, x11rb and the XDG portal backend of rfd.
  passthru.runtimeLibs = [
    libGL
    libxkbcommon
    wayland
    libxcb
    dbus
  ];

  # After the fixup phase, which strips rpath entries nothing links against.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath finalAttrs.passthru.runtimeLibs} \
      $out/bin/mpi_installer_gui
  '';

  meta = {
    description = "Native Linux installer for Tale of Two Wastelands and related MPI packages";
    homepage = "https://github.com/sulfurnitride/TTW_Linux_Installer";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "mpi_installer_gui";
  };
})
