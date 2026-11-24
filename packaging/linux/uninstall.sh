#!/bin/sh
set -eu

prefix=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
rm -f "$prefix/bin/saiki" \
    "$prefix/share/applications/saiki.desktop" \
    "$prefix/share/icons/hicolor/scalable/apps/saiki.svg" \
    "$prefix/share/licenses/saiki/LICENSE" \
    "$prefix/bin/saiki-uninstall"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi
echo 'Saiki has been uninstalled. Your settings have been kept.'
