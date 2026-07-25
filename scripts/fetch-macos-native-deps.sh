#!/usr/bin/env bash
# Fetches GTK 4 and its full runtime dependency closure for macOS (both arm64 and x64) from
# Homebrew's prebuilt bottles, rewrites each dylib's install names to be self-contained
# (@rpath-relative instead of an absolute Homebrew prefix path), verifies the resulting set is
# import-complete, bundles it into src/CSGtk.Gtk.Native.macOS/runtimes/osx-{arm64,x64}/native/,
# and packs the NuGet package into src/local-packages/.
#
# Rerun this whenever you want to refresh to a newer GTK4/Homebrew release - it always re-resolves
# the dependency graph from Homebrew's live formula API rather than a pinned list.
#
# IMPORTANT: this script must be run on an actual Mac. install_name_tool, otool, and codesign are
# part of the Xcode Command Line Tools (`xcode-select --install` if you don't have them) and have
# no equivalent on Linux/Windows - the install-name rewriting and re-signing this script does
# cannot be cross-compiled from another platform. (The dependency resolution and bottle
# download/extraction steps were prototyped and verified cross-platform; the otool/install_name_tool
# steps below have not been executed end-to-end anywhere but a real Mac can run them.)
#
# Prerequisites: curl, tar, python3, dotnet, and Xcode Command Line Tools (otool, install_name_tool,
# codesign - `xcode-select --install`).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT_DIR="$REPO_ROOT/src/CSGtk.Gtk.Native.macOS"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

ROOT_FORMULA="gtk4"
ROOT_DYLIB_GLOB="libgtk-4.*.dylib"

# (bottle tag, .NET RID) pairs. hicolor-icon-theme is the only closure member without an
# arch-specific tag (it's pure icon data); it's handled via the "all" fallback below.
FLAVORS=("arm64_tahoe:osx-arm64" "sonoma:osx-x64")

for tool in curl tar python3 otool install_name_tool codesign; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' is required but not found on PATH" >&2; exit 1; }
done

echo "==> Resolving Homebrew dependency closure for $ROOT_FORMULA..."
mkdir -p "$WORK_DIR/formula-json"
python3 - "$ROOT_FORMULA" "$WORK_DIR/formula-json" > "$WORK_DIR/closure.json" << 'PYEOF'
import json, os, sys, urllib.request

root, cache_dir = sys.argv[1], sys.argv[2]

def fetch_formula(name):
    cache_path = os.path.join(cache_dir, f"{name}.json")
    if os.path.exists(cache_path):
        return json.load(open(cache_path))
    url = f"https://formulae.brew.sh/api/formula/{name}.json"
    with urllib.request.urlopen(url, timeout=30) as resp:
        data = json.load(resp)
    json.dump(data, open(cache_path, 'w'))
    return data

seen = {}
queue = [root]
while queue:
    name = queue.pop()
    if name in seen:
        continue
    data = fetch_formula(name)
    seen[name] = data
    for dep in data.get('dependencies', []):
        if dep not in seen:
            queue.append(dep)

result = {}
for name, data in seen.items():
    files = data.get('bottle', {}).get('stable', {}).get('files', {})
    result[name] = {tag: info['url'] for tag, info in files.items()}
print(json.dumps(result, indent=2))
PYEOF

CLOSURE_COUNT=$(python3 -c "import json; print(len(json.load(open('$WORK_DIR/closure.json'))))")
echo "    $CLOSURE_COUNT packages in the closure."

get_token() {
    # Takes a ghcr.io repo path (e.g. "homebrew/core/glib" or "homebrew/core/icu4c/78" - versioned
    # formulae like icu4c@78 use a slash in the actual repo path, not "@", so this must be derived
    # from the bottle URL itself rather than assumed from the formula name).
    local repo_path="$1"
    curl -s "https://ghcr.io/token?service=ghcr.io&scope=repository:${repo_path}:pull" \
        | python3 -c "import json,sys; print(json.load(sys.stdin)['token'])"
}

repo_path_from_url() {
    # Extracts "homebrew/core/<formula>[/<version>]" from a
    # https://ghcr.io/v2/homebrew/core/<formula>/blobs/sha256:... URL.
    python3 -c "import sys, re; print(re.match(r'https://ghcr\.io/v2/(.+)/blobs/', sys.argv[1]).group(1))" "$1"
}

for flavor in "${FLAVORS[@]}"; do
    TAG="${flavor%%:*}"
    RID="${flavor##*:}"
    echo ""
    echo "=== $RID (bottle tag: $TAG) ==="

    FLAVOR_DIR="$WORK_DIR/$RID"
    DOWNLOAD_DIR="$FLAVOR_DIR/download"
    EXTRACT_DIR="$FLAVOR_DIR/extracted"
    STAGE_DIR="$FLAVOR_DIR/stage"
    mkdir -p "$DOWNLOAD_DIR" "$EXTRACT_DIR" "$STAGE_DIR"

    echo "==> Downloading bottles..."
    python3 -c "
import json
d = json.load(open('$WORK_DIR/closure.json'))
for name, tags in d.items():
    url = tags.get('$TAG') or tags.get('all')
    if url is None:
        raise SystemExit(f'no bottle for {name} matching tag $TAG or all')
    print(f'{name} {url}')
" | while read -r name url; do
        repo_path=$(repo_path_from_url "$url")
        token=$(get_token "$repo_path")
        curl -sL -H "Authorization: Bearer $token" "$url" -o "$DOWNLOAD_DIR/$name.tar.gz" --max-time 120
    done

    echo "==> Extracting bottles..."
    for pkg in "$DOWNLOAD_DIR"/*.tar.gz; do
        tar -xzf "$pkg" -C "$EXTRACT_DIR"
    done

    echo "==> Collecting dylibs..."
    # Each bottle extracts to <formula>/<version>/lib/*.dylib. Copy the real files (skip
    # unversioned convenience symlinks like "libfoo.dylib" -> "libfoo.1.dylib" - only the
    # versioned real file is what other libraries actually reference).
    find "$EXTRACT_DIR" -path "*/lib/*.dylib" -not -type l -exec cp {} "$STAGE_DIR/" \;
    echo "    $(ls "$STAGE_DIR" | wc -l | tr -d ' ') dylibs collected."

    ROOT_DYLIB=$(find "$STAGE_DIR" -maxdepth 1 -name "$ROOT_DYLIB_GLOB" | head -1)
    if [ -z "$ROOT_DYLIB" ]; then
        echo "error: no file matching $ROOT_DYLIB_GLOB found after extraction" >&2
        exit 1
    fi
    ROOT_DYLIB=$(basename "$ROOT_DYLIB")
    echo "    root dylib: $ROOT_DYLIB"

    echo "==> Rewriting install names to be self-contained (@rpath), re-signing..."
    for f in "$STAGE_DIR"/*.dylib; do
        name=$(basename "$f")
        install_name_tool -id "@rpath/$name" "$f"
        # Every LC_LOAD_DYLIB entry whose basename matches another file we bundled gets
        # rewritten to "@rpath/<that file>", regardless of what prefix it currently uses
        # (Homebrew's "@@HOMEBREW_PREFIX@@/opt/<pkg>/lib/..." placeholder, or an already-
        # substituted absolute path) - this is what makes the bundle location-independent.
        otool -L "$f" | tail -n +2 | awk '{print $1}' | while read -r dep; do
            depname=$(basename "$dep")
            if [ -f "$STAGE_DIR/$depname" ] && [ "$depname" != "$name" ]; then
                install_name_tool -change "$dep" "@rpath/$depname" "$f" 2>/dev/null || true
            fi
        done
        install_name_tool -add_rpath "@loader_path/." "$f" 2>/dev/null || true
        codesign --force -s - "$f" 2>/dev/null || true
    done

    echo "==> Verifying every import is bundled or a recognized OS-provided path..."
    python3 - "$STAGE_DIR" "$ROOT_DYLIB" << 'PYEOF'
import os, re, subprocess, sys

stage_dir, root = sys.argv[1], sys.argv[2]

def is_system_path(path):
    return path.startswith('/usr/lib/') or path.startswith('/System/Library/')

def imports_of(path):
    out = subprocess.run(['otool', '-L', path], capture_output=True, text=True).stdout
    lines = out.splitlines()[1:]
    return [line.strip().split(' ')[0] for line in lines if line.strip()]

seen = set()
missing = set()
queue = [root]
while queue:
    name = queue.pop()
    path = os.path.join(stage_dir, name)
    if not os.path.isfile(path):
        continue
    if name in seen:
        continue
    seen.add(name)
    for dep in imports_of(path):
        if dep.startswith('@rpath/'):
            depname = dep[len('@rpath/'):]
            if os.path.isfile(os.path.join(stage_dir, depname)):
                if depname not in seen:
                    queue.append(depname)
            else:
                missing.add(dep)
        elif not is_system_path(dep):
            missing.add(dep)

if missing:
    print("TEST FAILED: unresolved imports:", file=sys.stderr)
    for m in sorted(missing):
        print(f"  {m}", file=sys.stderr)
    sys.exit(1)

print(f"TEST PASSED: {len(seen)} dylibs, every import is bundled (@rpath) or OS-provided.")
PYEOF

    BUNDLE_DIR="$PROJECT_DIR/runtimes/$RID/native"
    echo "==> Installing bundle into $BUNDLE_DIR..."
    rm -rf "$BUNDLE_DIR"
    mkdir -p "$BUNDLE_DIR"
    cp "$STAGE_DIR"/*.dylib "$BUNDLE_DIR/"
    echo "    $(ls "$BUNDLE_DIR" | wc -l | tr -d ' ') files, $(du -sh "$BUNDLE_DIR" | cut -f1)"
done

echo ""
echo "==> Packing NuGet package..."
dotnet pack "$PROJECT_DIR/CSGtk.Gtk.Native.macOS.csproj" -c Release -o "$REPO_ROOT/src/local-packages"

echo "==> Done."
