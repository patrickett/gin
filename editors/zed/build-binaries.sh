#!/bin/bash
# Build ginlsp binaries for distribution

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
LSP_DIR="$PROJECT_ROOT/tools/ginlsp"
OUTPUT_DIR="$SCRIPT_DIR/bin"

echo "Building ginlsp binaries..."
echo "Project root: $PROJECT_ROOT"
echo "Output directory: $OUTPUT_DIR"

mkdir -p "$OUTPUT_DIR"

# Get current system architecture
ARCH="$(uname -m)"

build_target() {
    local target=$1
    local output_subdir=$2

    echo "Building for $target..."

    cargo build --release --manifest-path "$LSP_DIR/Cargo.toml" --target "$target"

    mkdir -p "$OUTPUT_DIR/$output_subdir"

    if [[ "$target" == *"windows"* ]]; then
        cp "$PROJECT_ROOT/target/$target/release/ginlsp.exe" "$OUTPUT_DIR/$output_subdir/"
    else
        cp "$PROJECT_ROOT/target/$target/release/ginlsp" "$OUTPUT_DIR/$output_subdir/"
    fi
}

# Build only for the current system architecture
if [[ "$ARCH" == "arm64" ]]; then
    build_target "aarch64-apple-darwin" "aarch64-apple-darwin"
    echo "Built for macOS ARM64 (Apple Silicon)"
elif [[ "$ARCH" == "x86_64" ]]; then
    build_target "x86_64-apple-darwin" "x86_64-apple-darwin"
    echo "Built for macOS Intel (x86_64)"
else
    echo "Unsupported architecture: $ARCH"
    exit 1
fi

echo "Build complete!"
ls -la "$OUTPUT_DIR"
