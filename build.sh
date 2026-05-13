#!/bin/bash
set -e

# ── Detect project root (where this script lives) ──
PROJECT_ROOT="$(cd "$(dirname "$0")" && pwd)"

# ── Read project name from tauri.conf.json (fallback to package.json) ──
PROJECT_NAME=""
if [ -f "$PROJECT_ROOT/src-tauri/tauri.conf.json" ]; then
    PROJECT_NAME=$(grep -oP '"productName"\s*:\s*"\K[^"]*' "$PROJECT_ROOT/src-tauri/tauri.conf.json" | head -1)
fi
if [ -z "$PROJECT_NAME" ]; then
    PROJECT_NAME=$(grep -oP '"name"\s*:\s*"\K[^"]*' "$PROJECT_ROOT/package.json" | head -1)
fi

# ── Read version from tauri.conf.json ──
VERSION="0.0.0"
if [ -f "$PROJECT_ROOT/src-tauri/tauri.conf.json" ]; then
    VERSION=$(grep -oP '"version"\s*:\s*"\K[^"]*' "$PROJECT_ROOT/src-tauri/tauri.conf.json" | head -1)
fi
VERSION="${VERSION:-0.0.0}"

# ── Read binary name from Cargo.toml ──
# Try [[bin]] name first, fall back to [package] name
BINARY_NAME=""
if [ -f "$PROJECT_ROOT/src-tauri/Cargo.toml" ]; then
    # Check for [[bin]] section
    BIN_SECTION=$(awk '/^\[\[bin\]\]/{found=1} found && /^name/{print; exit}' "$PROJECT_ROOT/src-tauri/Cargo.toml")
    if [ -n "$BIN_SECTION" ]; then
        BINARY_NAME=$(echo "$BIN_SECTION" | grep -oP '"\K[^"]*')
    fi
    if [ -z "$BINARY_NAME" ]; then
        BINARY_NAME=$(grep -oP '^name\s*=\s*"\K[^"]*' "$PROJECT_ROOT/src-tauri/Cargo.toml" | head -1)
    fi
fi
BINARY_NAME="${BINARY_NAME:-$PROJECT_NAME}"

# ── Architecture ──
ARCH=$(uname -m)

# ── Paths ──
SRC_TAURI="$PROJECT_ROOT/src-tauri"
BINARY="$SRC_TAURI/target/release/$BINARY_NAME"
DEB_DIR="$SRC_TAURI/target/release/bundle/deb"
RPM_DIR="$SRC_TAURI/target/release/bundle/rpm"
BUILD_DIR="$PROJECT_ROOT/build"
TAR_NAME="${PROJECT_NAME}_${VERSION}_${ARCH}.tar.gz"

echo "========================================"
echo "  Tauri Build Script"
echo "  Project : $PROJECT_NAME"
echo "  Version : $VERSION"
echo "  Binary  : $BINARY_NAME"
echo "  Arch    : $ARCH"
echo "========================================"

# ── Step 1: Build with Tauri ──
echo ""
echo "==> Building release with Tauri..."
cd "$PROJECT_ROOT"
npm run tauri build

# ── Step 2: Prepare build directory ──
echo ""
echo "==> Preparing build directory..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# ── Step 3: Copy deb/rpm packages ──
echo "==> Copying packages..."
if ls "$DEB_DIR"/*.deb 1>/dev/null 2>&1; then
    for deb in "$DEB_DIR"/*.deb; do
        cp "$deb" "$BUILD_DIR/"
        echo "  -> $(basename "$deb")"
    done
else
    echo "  (no .deb packages found)"
fi

if ls "$RPM_DIR"/*.rpm 1>/dev/null 2>&1; then
    for rpm in "$RPM_DIR"/*.rpm; do
        cp "$rpm" "$BUILD_DIR/"
        echo "  -> $(basename "$rpm")"
    done
else
    echo "  (no .rpm packages found)"
fi

# ── Step 4: Create portable tar.gz ──
echo ""
echo "==> Creating portable tar.gz..."
TAR_DIR="$BUILD_DIR/${PROJECT_NAME}_${VERSION}_${ARCH}"
mkdir -p "$TAR_DIR"

if [ -f "$BINARY" ]; then
    cp "$BINARY" "$TAR_DIR/"
else
    echo "  WARNING: Binary not found at $BINARY"
fi

cat > "$TAR_DIR/README.txt" << EOF
$PROJECT_NAME v$VERSION - Portable (Linux $ARCH)

Requirements: webkit2gtk-4.1, gtk3

Usage:
  ./$BINARY_NAME

EOF

cd "$BUILD_DIR"
tar -czf "$TAR_NAME" "$(basename "$TAR_DIR")"
rm -rf "$TAR_DIR"

echo "  -> $TAR_NAME"

# ── Done ──
echo ""
echo "========================================"
echo "  Build complete! Output in: $BUILD_DIR"
echo "========================================"
ls -lh "$BUILD_DIR"
