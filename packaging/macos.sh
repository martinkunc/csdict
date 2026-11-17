#!/usr/bin/env bash
# Packages a macOS build: packaging/macos.sh <csdict binary> <version> <out.dmg>
#
# macOS only treats a *.app directory with this Contents/{MacOS,Resources,Info.plist} layout as a
# double-clickable application. Ad-hoc signing (no Apple Developer certificate) avoids the "app is
# damaged" error; without notarization the first start still needs right-click -> Open.
set -euo pipefail
bin="$1"
version="$2"
out="$3"
root="$(cd "$(dirname "$0")/.." && pwd)"
stage="$(mktemp -d)"
app="$stage/CSDict.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$bin" "$app/Contents/MacOS/csdict"
cp "$root/assets/icon/icon.icns" "$app/Contents/Resources/AppIcon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>CSDict</string>
  <key>CFBundleDisplayName</key><string>CSDict</string>
  <key>CFBundleIdentifier</key><string>dev.csdict.CSDict</string>
  <key>CFBundleExecutable</key><string>csdict</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.reference</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --deep -s - "$app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname CSDict -srcfolder "$stage" -ov -format UDZO "$out"
rm -rf "$stage"
echo "wrote $out"
