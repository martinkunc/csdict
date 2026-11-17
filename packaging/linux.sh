#!/usr/bin/env bash
# Packages a Linux build: packaging/linux.sh <csdict binary> <out.tar.gz>
#
# A self-contained tarball (no .deb/.rpm), so the icon and menu entry ship in the standard XDG
# layout plus a script that installs them for the current user. Wayland/X11 match the window to
# the .desktop file by the app id "dev.csdict.CSDict" (set in main.rs).
set -euo pipefail
bin="$1"
out="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
stage="$(mktemp -d)"
dest="$stage/CSDict"
mkdir -p "$dest/share/applications"
install -m 755 "$bin" "$dest/csdict"
cp "$root/README.md" "$dest/"
cp "$root/assets/icon/dev.csdict.CSDict.desktop" "$dest/share/applications/"
for size in 16 24 32 48 64 128 256 512; do
  mkdir -p "$dest/share/icons/hicolor/${size}x${size}/apps"
  cp "$root/assets/icon/png/icon-$size.png" "$dest/share/icons/hicolor/${size}x${size}/apps/dev.csdict.CSDict.png"
done
cat > "$dest/install-desktop-icon.sh" <<'SCRIPT'
#!/bin/sh
# Installs the CSDict icon and menu entry for the current user.
set -e
dir="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/.local/share/icons" "$HOME/.local/share/applications"
cp -r "$dir/share/icons/." "$HOME/.local/share/icons/"
sed "s|^Exec=.*|Exec=\"$dir/csdict\"|" "$dir/share/applications/dev.csdict.CSDict.desktop" \
  > "$HOME/.local/share/applications/dev.csdict.CSDict.desktop"
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$HOME/.local/share/applications" || true
echo "Installed. CSDict is now in your application menu."
SCRIPT
chmod +x "$dest/install-desktop-icon.sh"
tar -czf "$out" -C "$stage" CSDict
rm -rf "$stage"
echo "wrote $out"
