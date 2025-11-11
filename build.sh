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

    # Clean project build
    dotnet clean "$GUI_PROJECT" -c Release > /dev/null 2>&1 || true

    # Remove old release directory
    if [ -d "$OUTPUT_DIR" ]; then
        rm -rf "$OUTPUT_DIR"
        print_success "Removed old release directory"
    fi

    # Remove old archive
    if [ -f "universal-mpi-installer-linux-x64.tar.gz" ]; then
        rm -f "universal-mpi-installer-linux-x64.tar.gz"
        print_success "Removed old archive"
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

package_release() {
    print_info "Packaging release..."

    # Create release directory
    mkdir -p "$OUTPUT_DIR"

    # Copy all files from GUI publish directory
    cp -r TtwInstallerGui/bin/Release/net9.0/$RUNTIME/publish/* "$OUTPUT_DIR/"

    # Make executables executable
    chmod +x "$OUTPUT_DIR/ttw_linux_gui"
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
    echo "  • Release:  $OUTPUT_DIR/"
    echo "  • Archive:  universal-mpi-installer-linux-x64.tar.gz"
    echo ""
    echo "To run:"
    echo "  cd $OUTPUT_DIR"
    echo "  ./ttw_linux_gui"
    echo ""
    echo "Files in release directory:"
    ls -lh "$OUTPUT_DIR" | grep -v "^total" | grep -v "^d" | awk '{print "  • " $9 " (" $5 ")"}'
}

show_help() {
    echo "Usage: ./build.sh [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -h, --help       Show this help message"
    echo "  -c, --clean      Clean build artifacts and exit"
    echo "  --no-archive     Skip creating release archive"
    echo ""
    echo "Examples:"
    echo "  ./build.sh              # Full build with archive"
    echo "  ./build.sh --no-archive # Build without creating .tar.gz"
    echo "  ./build.sh --clean      # Clean all build artifacts"
}

# Parse arguments
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

# Build (GUI includes everything)
build_gui

echo ""
package_release

if [ "$NO_ARCHIVE" = false ]; then
    echo ""
    create_archive
fi

echo ""
show_summary
