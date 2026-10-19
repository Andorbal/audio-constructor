#!/bin/sh
# Installs Audio Constructor from this folder for you alone, without root:
#
#   ./install.sh               install, or update an earlier install
#   ./install.sh --uninstall   remove what it installed
#
# Where things go (override with the variables in brackets):
#   the editor and audioc         ~/.local/bin                      [PREFIX=~/.local]
#   examples                      ~/.local/share/audio-constructor
#   menu entry, icons, file types ~/.local/share/{applications,icons,mime}
#   plugins                       ~/.clap and ~/.vst3               [CLAP_DIR, VST3_DIR]
#
# The .deb package installs the same files system-wide instead.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}
clap_dir=${CLAP_DIR:-$HOME/.clap}
vst3_dir=${VST3_DIR:-$HOME/.vst3}
share=$prefix/share
name="Audio Constructor"
sizes="16 32 48 64 128 256 512"

refresh() {
    # Menus and file managers notice new entries without these; they just
    # notice sooner with them.
    command -v update-desktop-database > /dev/null 2>&1 && update-desktop-database -q "$share/applications" || true
    command -v update-mime-database > /dev/null 2>&1 && update-mime-database "$share/mime" > /dev/null 2>&1 || true
    command -v gtk-update-icon-cache > /dev/null 2>&1 && gtk-update-icon-cache -q -t "$share/icons/hicolor" || true
}

uninstall() {
    rm -f "$prefix/bin/audio-constructor" "$prefix/bin/audioc"
    rm -rf "$share/audio-constructor"
    rm -f "$share/applications/audio-constructor.desktop"
    rm -f "$share/mime/packages/audio-constructor.xml"
    for s in $sizes; do
        rm -f "$share/icons/hicolor/${s}x${s}/apps/audio-constructor.png"
    done
    rm -f "$clap_dir/$name.clap"
    rm -rf "$vst3_dir/$name.vst3"
    refresh
    echo "Removed $name."
}

if [ "${1:-}" = "--uninstall" ]; then
    uninstall
    exit 0
elif [ $# -gt 0 ]; then
    sed -n '2,12s/^# \{0,1\}//p' "$0"
    exit 2
fi

mkdir -p "$prefix/bin" "$share/applications" "$share/mime/packages" "$clap_dir" "$vst3_dir"
install -m 755 "$here/audio-constructor" "$here/audioc" "$prefix/bin/"
rm -rf "$share/audio-constructor"
mkdir -p "$share/audio-constructor"
cp -R "$here/examples" "$share/audio-constructor/"
install -m 644 "$here/share/audio-constructor.desktop" "$share/applications/"
install -m 644 "$here/share/audio-constructor.xml" "$share/mime/packages/"
for s in $sizes; do
    mkdir -p "$share/icons/hicolor/${s}x${s}/apps"
    install -m 644 "$here/share/icons/icon-$s.png" "$share/icons/hicolor/${s}x${s}/apps/audio-constructor.png"
done
install -m 755 "$here/plugins/$name.clap" "$clap_dir/"
rm -rf "$vst3_dir/$name.vst3"
cp -R "$here/plugins/$name.vst3" "$vst3_dir/"
refresh

echo "Installed $name $(cat "$here/VERSION" 2> /dev/null || true)."
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Add $prefix/bin to your PATH to run audioc from a terminal." ;;
esac
