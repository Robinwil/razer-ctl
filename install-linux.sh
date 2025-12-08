#!/bin/bash
# Installation script for razer-ctl on Linux

set -e

echo "=== Razer Control Utility - Linux Installation ==="

# Check if running as root for udev rules installation
NEED_SUDO=""
if [ "$EUID" -ne 0 ]; then
    NEED_SUDO="sudo"
    echo "Note: Some steps require sudo privileges"
fi

# Install udev rules
echo ""
echo "[1/4] Installing udev rules..."
$NEED_SUDO cp 99-razer.rules /etc/udev/rules.d/99-razer.rules
$NEED_SUDO udevadm control --reload-rules
$NEED_SUDO udevadm trigger
echo "✓ udev rules installed"

# Add user to plugdev group
echo ""
echo "[2/4] Adding user to plugdev group..."
if ! groups | grep -q plugdev; then
    $NEED_SUDO usermod -aG plugdev $USER
    echo "✓ User added to plugdev group (logout required to take effect)"
else
    echo "✓ User already in plugdev group"
fi

# Check for required dependencies
echo ""
echo "[3/4] Checking dependencies..."
MISSING_DEPS=""

# Check for libhidapi
if ! ldconfig -p | grep -q libhidapi; then
    MISSING_DEPS="$MISSING_DEPS libhidapi-dev"
fi

# Check for GTK3 (for tray application)
if ! pkg-config --exists gtk+-3.0 2>/dev/null; then
    MISSING_DEPS="$MISSING_DEPS libgtk-3-dev"
fi

# Check for libappindicator (for system tray)
if ! pkg-config --exists appindicator3-0.1 2>/dev/null && ! pkg-config --exists ayatana-appindicator3-0.1 2>/dev/null; then
    MISSING_DEPS="$MISSING_DEPS libayatana-appindicator3-dev"
fi

if [ -n "$MISSING_DEPS" ]; then
    echo "Missing dependencies detected. Install with:"
    echo ""
    echo "  # Debian/Ubuntu:"
    echo "  sudo apt install$MISSING_DEPS libudev-dev pkg-config"
    echo ""
    echo "  # Fedora:"
    echo "  sudo dnf install hidapi-devel gtk3-devel libappindicator-gtk3-devel systemd-devel"
    echo ""
    echo "  # Arch Linux:"
    echo "  sudo pacman -S hidapi gtk3 libappindicator-gtk3"
    echo ""
else
    echo "✓ All dependencies found"
fi

# Build instructions
echo ""
echo "[4/4] Build instructions..."
echo ""
echo "To build the project, run:"
echo ""
echo "  cargo build --release"
echo ""
echo "Binaries will be in target/release/"
echo "  - razer-cli    : Command-line interface"
echo "  - razer-tray   : System tray application"
echo ""

# Desktop entry for tray app
echo "Optional: Create desktop entry for auto-start?"
echo "  mkdir -p ~/.config/autostart"
echo "  cat > ~/.config/autostart/razer-tray.desktop << EOF"
echo "[Desktop Entry]"
echo "Type=Application"
echo "Name=Razer Control Tray"
echo "Exec=$(pwd)/target/release/razer-tray"
echo "Icon=razer"
echo "Terminal=false"
echo "Categories=Utility;"
echo "X-GNOME-Autostart-enabled=true"
echo "EOF"
echo ""

echo "=== Installation complete ==="
echo ""
echo "NOTE: You may need to log out and back in for group changes to take effect."
echo "      Then reconnect/replug your Razer device."
