#!/usr/bin/env bash
set -e

# Universal MPI Installer - Build Script
# Builds single-file executables for Linux

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
GUI_PROJECT="TtwInstallerGui/TtwInstallerGui.csproj"
CLI_PROJECT="TtwInstaller/TtwInstaller.csproj"
OUTPUT_DIR="release"
RUNTIME="linux-x64"

print_header() {
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  Universal MPI Installer - Build${NC}"
    echo -e "${BLUE}========================================${NC}"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_info() {
    echo -e "${YELLOW}→${NC} $1"
}

check_dependencies() {
    print_info "Checking dependencies..."

    if ! command -v dotnet &> /dev/null; then
        print_error "dotnet SDK not found!"
        echo "Please install .NET 9.0 SDK from: https://dotnet.microsoft.com/download"
        exit 1
    fi

    DOTNET_VERSION=$(dotnet --version)
    print_success "Found dotnet SDK: $DOTNET_VERSION"
}

clean_build() {
    print_info "Cleaning previous builds..."

    # Clean project builds
    dotnet clean "$GUI_PROJECT" -c Release > /dev/null 2>&1 || true
    dotnet clean "$CLI_PROJECT" -c Release > /dev/null 2>&1 || true

    # Remove old release directory
    if [ -d "$OUTPUT_DIR" ]; then
        rm -rf "$OUTPUT_DIR"
        print_success "Removed old release directory"
    fi

    print_success "Clean complete"
}

build_gui() {
    print_info "Building GUI (ttw_linux_gui)..."
    echo ""

    dotnet publish "$GUI_PROJECT" \
        -c Release \
        -r "$RUNTIME" \
        --self-contained true \
        -p:PublishSingleFile=true \
        -p:IncludeNativeLibrariesForSelfExtract=true \
        -p:EnableCompressionInSingleFile=true \
        --nologo

    echo ""
    print_success "GUI build complete"
}

build_cli() {
    print_info "Building CLI (TtwInstaller)..."
    echo ""

    dotnet publish "$CLI_PROJECT" \
        -c Release \
        -r "$RUNTIME" \
        --self-contained true \
        -p:PublishSingleFile=true \
        -p:IncludeNativeLibrariesForSelfExtract=true \
        -p:EnableCompressionInSingleFile=true \
        --nologo

    echo ""
    print_success "CLI build complete"
}

package_release() {
    print_info "Packaging release..."

    # Create release directory
    mkdir -p "$OUTPUT_DIR"

    # Copy GUI files
    cp TtwInstallerGui/bin/Release/net9.0/$RUNTIME/publish/ttw_linux_gui "$OUTPUT_DIR/"
    cp TtwInstallerGui/bin/Release/net9.0/$RUNTIME/publish/libbsa_capi.so "$OUTPUT_DIR/"
    cp TtwInstallerGui/bin/Release/net9.0/$RUNTIME/publish/xdelta3 "$OUTPUT_DIR/"

    # Copy CLI files
    cp TtwInstaller/bin/Release/net9.0/$RUNTIME/publish/TtwInstaller "$OUTPUT_DIR/"

    # Copy documentation
    cp README.md "$OUTPUT_DIR/"
    cp TtwInstaller/mpi-config.json.example "$OUTPUT_DIR/"

    # Make executables executable
    chmod +x "$OUTPUT_DIR/ttw_linux_gui"
    chmod +x "$OUTPUT_DIR/TtwInstaller"
    chmod +x "$OUTPUT_DIR/xdelta3"

    print_success "Release packaged in $OUTPUT_DIR/"
}

create_archive() {
    print_info "Creating release archive..."

    ARCHIVE_NAME="universal-mpi-installer-linux-x64.tar.gz"

    cd "$OUTPUT_DIR"
    tar -czf "../$ARCHIVE_NAME" ./*
    cd ..

    SIZE=$(du -h "$ARCHIVE_NAME" | cut -f1)
    print_success "Created $ARCHIVE_NAME ($SIZE)"
}

show_summary() {
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Build Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo "Build outputs:"
    echo "  • GUI:     $OUTPUT_DIR/ttw_linux_gui"
    echo "  • CLI:     $OUTPUT_DIR/TtwInstaller"
    echo "  • Archive: universal-mpi-installer-linux-x64.tar.gz"
    echo ""
    echo "To run:"
    echo "  cd $OUTPUT_DIR"
    echo "  ./ttw_linux_gui"
    echo ""
    echo "File sizes:"
    ls -lh "$OUTPUT_DIR" | grep -E "(ttw_linux_gui|TtwInstaller|libbsa_capi|xdelta3)" | awk '{print "  • " $9 ": " $5}'
}

show_help() {
    echo "Usage: ./build.sh [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -h, --help       Show this help message"
    echo "  -c, --clean      Clean build artifacts and exit"
    echo "  --gui-only       Build only the GUI"
    echo "  --cli-only       Build only the CLI"
    echo "  --no-archive     Skip creating release archive"
    echo ""
    echo "Examples:"
    echo "  ./build.sh              # Full build with archive"
    echo "  ./build.sh --gui-only   # Build only GUI"
    echo "  ./build.sh --clean      # Clean all build artifacts"
}

# Parse arguments
GUI_ONLY=false
CLI_ONLY=false
NO_ARCHIVE=false
CLEAN_ONLY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_help
            exit 0
            ;;
        -c|--clean)
            CLEAN_ONLY=true
            shift
            ;;
        --gui-only)
            GUI_ONLY=true
            shift
            ;;
        --cli-only)
            CLI_ONLY=true
            shift
            ;;
        --no-archive)
            NO_ARCHIVE=true
            shift
            ;;
        *)
            print_error "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac
done

# Main execution
print_header
echo ""

check_dependencies
echo ""

clean_build
echo ""

if [ "$CLEAN_ONLY" = true ]; then
    print_success "Clean complete. Exiting."
    exit 0
fi

# Build based on flags
if [ "$CLI_ONLY" = true ]; then
    build_cli
elif [ "$GUI_ONLY" = true ]; then
    build_gui
else
    build_gui
    echo ""
    build_cli
fi

echo ""
package_release

if [ "$NO_ARCHIVE" = false ]; then
    echo ""
    create_archive
fi

echo ""
show_summary
