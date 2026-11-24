#!/bin/sh
set -eu

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix="$HOME/.local"

if [ ! -x "$source_dir/saiki" ]; then
    echo 'Run this installer from the extracted Saiki download.' >&2
    exit 1
fi

install -Dm755 "$source_dir/saiki" "$prefix/bin/saiki"
install -Dm755 "$source_dir/uninstall.sh" "$prefix/bin/saiki-uninstall"
install -Dm644 "$source_dir/saiki.svg" "$prefix/share/icons/hicolor/scalable/apps/saiki.svg"
install -Dm644 "$source_dir/LICENSE" "$prefix/share/licenses/saiki/LICENSE"
mkdir -p "$prefix/share/applications"
# Desktop entry quoted arguments require escaping both the string and Exec layers.
exec_path=$(printf '%s' "$prefix/bin/saiki" | sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g; s/%/%%/g')
while IFS= read -r line; do
    case "$line" in
        Exec=*) printf 'Exec="%s"\n' "$exec_path" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$source_dir/saiki.desktop" > "$prefix/share/applications/saiki.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi
printf 'Saiki is installed. Open Saiki from your application launcher.\nTo uninstall, run: "%s/bin/saiki-uninstall"\n' "$prefix"
