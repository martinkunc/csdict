#!/usr/bin/env bash
# Fetches GTK 4 and its full runtime dependency closure for Windows x64 from MSYS2's UCRT64
# repository, verifies the resulting DLL set is import-complete, bundles it into
# src/CSDict.Gtk.Native.Windows/runtimes/win-x64/native/, and packs the NuGet package into
# src/local-packages/.
#
# Rerun this whenever you want to refresh to a newer GTK4/MSYS2 release - it always re-resolves
# the dependency graph from the live repository rather than a pinned list.
#
# Prerequisites: curl, tar, zstd, objdump (binutils), python3, dotnet.
#   - On an actual Windows box, the easiest way to get all of these at once is to run this
#     script from an MSYS2 UCRT64 shell (https://www.msys2.org/): `pacman -S curl tar zstd
#     binutils python3` (all normally already present in a base MSYS2 install except zstd).
#   - This also works unmodified on Linux/macOS/WSL purely to cross-fetch the Windows binaries
#     for packaging - nothing here executes Windows code, it only downloads and rearranges files.
#   - If you'd rather not touch MSYS2/bash on a native Windows machine at all, use the PowerShell
#     equivalent: fetch-windows-native-deps.ps1. That version bundles the full MSYS2 dependency
#     set unfiltered (no objdump available on a vanilla Windows box to trim it down), so it
#     produces a larger but equally correct bundle.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT_DIR="$REPO_ROOT/src/CSDict.Gtk.Native.Windows"
BUNDLE_DIR="$PROJECT_DIR/runtimes/win-x64/native"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

MSYS2_REPO_URL="https://repo.msys2.org/mingw/ucrt64"
MSYS2_PREFIX="mingw-w64-ucrt-x86_64-"
ROOT_PACKAGE="${MSYS2_PREFIX}gtk4"
ROOT_DLL="libgtk-4-1.dll"

for tool in curl tar zstd objdump python3; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' is required but not found on PATH" >&2; exit 1; }
done

echo "==> Downloading MSYS2 UCRT64 package database..."
curl -sL "$MSYS2_REPO_URL/ucrt64.db" -o "$WORK_DIR/ucrt64.db"
mkdir -p "$WORK_DIR/db"
zstd -dc "$WORK_DIR/ucrt64.db" | tar -x -C "$WORK_DIR/db"

echo "==> Resolving dependency closure for $ROOT_PACKAGE..."
python3 - "$WORK_DIR/db" "$ROOT_PACKAGE" "$MSYS2_PREFIX" > "$WORK_DIR/closure.json" << 'PYEOF'
import os, re, sys, json

db_dir, root_pkg, prefix = sys.argv[1], sys.argv[2], sys.argv[3]

pkgs = {}
provides = {}
for d in os.listdir(db_dir):
    desc_path = os.path.join(db_dir, d, 'desc')
    if not os.path.isfile(desc_path):
        continue
    content = '\n' + open(desc_path).read()
    sections = re.split(r'\n%([A-Z0-9_]+)%\n', content)
    data = {}
    it = iter(sections[1:])
    for key, val in zip(it, it):
        data[key] = [v for v in val.strip('\n').split('\n') if v]
    name = data.get('NAME', [None])[0]
    if not name:
        continue
    pkgs[name] = {
        'filename': data.get('FILENAME', [None])[0],
        'depends': [re.split(r'[<>=]', dep)[0] for dep in data.get('DEPENDS', [])],
    }
    for prov in data.get('PROVIDES', []):
        provides[re.split(r'[<>=]', prov)[0]] = name

def resolve(name):
    if name in pkgs:
        return name
    return provides.get(name)

seen = set()
queue = [root_pkg]
missing = set()
while queue:
    cur = queue.pop()
    real = resolve(cur)
    if real is None:
        # dependencies outside the mingw-w64-ucrt-x86_64-* tree (e.g. plain "python") aren't
        # part of this repo and can't be resolved the same way; only flag same-prefix misses.
        if cur.startswith(prefix):
            missing.add(cur)
        continue
    if real in seen:
        continue
    seen.add(real)
    for dep in pkgs[real]['depends']:
        if resolve(dep) not in seen:
            queue.append(dep)

print(json.dumps({
    'filenames': [pkgs[n]['filename'] for n in sorted(seen) if pkgs[n]['filename']],
    'missing': sorted(missing),
}, indent=2))
PYEOF

MISSING_COUNT=$(python3 -c "import json; print(len(json.load(open('$WORK_DIR/closure.json'))['missing']))")
if [ "$MISSING_COUNT" -ne 0 ]; then
    echo "error: could not resolve these packages in the MSYS2 repo:" >&2
    python3 -c "import json; print('\n'.join(json.load(open('$WORK_DIR/closure.json'))['missing']))" >&2
    exit 1
fi

CANDIDATE_COUNT=$(python3 -c "import json; print(len(json.load(open('$WORK_DIR/closure.json'))['filenames']))")
echo "==> Downloading $CANDIDATE_COUNT candidate packages..."
mkdir -p "$WORK_DIR/download" "$WORK_DIR/extracted"
python3 -c "import json; print('\n'.join(json.load(open('$WORK_DIR/closure.json'))['filenames']))" | \
while read -r filename; do
    [ -z "$filename" ] && continue
    curl -sL "$MSYS2_REPO_URL/$filename" -o "$WORK_DIR/download/$filename" --max-time 120
done

echo "==> Extracting packages..."
for pkg in "$WORK_DIR"/download/*.pkg.tar.zst; do
    zstd -dc "$pkg" | tar -x -C "$WORK_DIR/extracted"
done

BIN_DIR="$WORK_DIR/extracted/ucrt64/bin"
if [ ! -f "$BIN_DIR/$ROOT_DLL" ]; then
    echo "error: $ROOT_DLL not found after extraction - MSYS2 package layout may have changed" >&2
    exit 1
fi

echo "==> Computing the real PE import closure from $ROOT_DLL (trims candidates down to what's actually linked)..."
python3 - "$BIN_DIR" "$ROOT_DLL" > "$WORK_DIR/pe_closure.json" << 'PYEOF'
import os, re, subprocess, sys, json

bindir, root = sys.argv[1], sys.argv[2]

# Windows/UCRT system DLLs that ship with the OS itself (Windows 10+) - never bundled, and not
# treated as a gap if absent from our extracted files.
SYSTEM_DLL_PATTERNS = [
    re.compile(r'^api-ms-win-', re.I),
    re.compile(r'^ext-ms-', re.I),
    re.compile(r'^winspool\.drv$', re.I),
    re.compile(r'^(kernel32|user32|gdi32|advapi32|ole32|oleaut32|shell32|shlwapi|ws2_32|winmm|'
               r'imm32|comctl32|comdlg32|crypt32|secur32|bcrypt|ncrypt|dwmapi|dxgi|d3d9|d3d11|d3d12|'
               r'd2d1|dwrite|wtsapi32|version|dnsapi|iphlpapi|netapi32|powrprof|propsys|rpcrt4|'
               r'setupapi|userenv|uxtheme|hid|mf|mfplat|avrt|ntdll|msvcrt|gdiplus|'
               r'opengl32|glu32|dcomp|uiautomationcore|ucrtbase|normaliz|wldap32|winhttp|urlmon|'
               r'mswsock|psapi|cabinet|windowscodecs|magnification|dbghelp|cfgmgr32|'
               r'msimg32|shcore|usp10)\.dll$', re.I),
]

def is_system_dll(name):
    return any(p.match(name) for p in SYSTEM_DLL_PATTERNS)

def file_for(name):
    path = os.path.join(bindir, name)
    if os.path.isfile(path):
        return path
    lname = name.lower()
    for f in os.listdir(bindir):
        if f.lower() == lname:
            return os.path.join(bindir, f)
    return None

def imports_of(path):
    out = subprocess.run(['objdump', '-p', path], capture_output=True, text=True).stdout
    return re.findall(r'DLL Name: (\S+)', out)

seen_files = set()
missing = set()
queue = [root]
while queue:
    name = queue.pop()
    path = file_for(name)
    if path is None:
        if not is_system_dll(name):
            missing.add(name)
        continue
    real_name = os.path.basename(path)
    if real_name in seen_files:
        continue
    seen_files.add(real_name)
    for dep in imports_of(path):
        queue.append(dep)

print(json.dumps({'closure': sorted(seen_files), 'missing': sorted(missing)}, indent=2))
PYEOF

PE_MISSING=$(python3 -c "import json; print(len(json.load(open('$WORK_DIR/pe_closure.json'))['missing']))")
if [ "$PE_MISSING" -ne 0 ]; then
    echo "TEST FAILED: these imports are neither bundled nor recognized OS/UCRT system DLLs:" >&2
    python3 -c "import json; print('\n'.join(json.load(open('$WORK_DIR/pe_closure.json'))['missing']))" >&2
    exit 1
fi

CLOSURE_COUNT=$(python3 -c "import json; print(len(json.load(open('$WORK_DIR/pe_closure.json'))['closure']))")
echo "==> TEST PASSED: $CLOSURE_COUNT DLLs, every import satisfied by the bundle or a system DLL."

echo "==> Installing bundle into $BUNDLE_DIR..."
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR"
python3 -c "import json; print('\n'.join(json.load(open('$WORK_DIR/pe_closure.json'))['closure']))" | \
while read -r name; do
    cp "$BIN_DIR/$name" "$BUNDLE_DIR/"
done
echo "    $(ls "$BUNDLE_DIR" | wc -l) files, $(du -sh "$BUNDLE_DIR" | cut -f1)"

echo "==> Packing NuGet package..."
dotnet pack "$PROJECT_DIR/CSDict.Gtk.Native.Windows.csproj" -c Release -o "$REPO_ROOT/src/local-packages"

echo "==> Done."
