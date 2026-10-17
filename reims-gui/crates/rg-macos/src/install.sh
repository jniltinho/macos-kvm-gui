#!/bin/sh
# macos-kvm-gui: install macOS from the full installer disc, with no network.
# sh INSTALL.SH [-u] [/Volumes/Target]   -u: unattended (erases the empty disk, accepts the license)
set -e
export LC_ALL=C
U=
if [ "$1" = -u ]; then U=1; shift; fi
T="${1:-/Volumes/Macintosh HD}"
T="${T%/}"
# Big Sur's recovery has no dirname or basename: the shell's own expansions instead.
case "$0" in */*) D="${0%/*}" ;; *) D=. ;; esac
N="${T##*/}"
if [ -z "$N" ]; then echo "Give a volume such as \"/Volumes/Macintosh HD\""; exit 1; fi
if [ -n "$U" ] && [ ! -d "$T" ]; then
  # The VM's new disk: the only internal disk whose first MiB reads back whole and all zeros.
  # A disk with any partition map, file system or the installer disc has data there; a disk
  # that cannot be read is skipped, never erased.
  E=
  for d in $(diskutil list internal physical | awk '/^\/dev\/disk[0-9]+/ {print $1}'); do
    # One read into a sample: it must succeed and give the whole MiB (n), with no byte but
    # zero (z). The recovery has no cmp; tr reads bytes only with LC_ALL=C.
    S=/tmp/mkg-sample
    rm -f "$S"
    if dd if="$d" of="$S" bs=1048576 count=1 2>/dev/null; then
      n=$(wc -c < "$S" | tr -d ' ')
      z=$(tr -d '\000' < "$S" | wc -c | tr -d ' ')
      if [ "$n" = 1048576 ] && [ "$z" = 0 ]; then E="$E $d"; fi
    fi
    rm -f "$S"
  done
  set -- $E
  if [ $# -ne 1 ]; then
    echo "Found $# empty disks, not one: erase the disk in Disk Utility as APFS named \"$N\" and run this again"
    exit 1
  fi
  echo "== Erasing $1 as APFS \"$N\""
  diskutil eraseDisk APFS "$N" GPT "$1"
fi
if [ ! -d "$T" ]; then
  echo "No volume at $T. Erase the disk in Disk Utility as APFS named \"Macintosh HD\", or run: sh $0 \"/Volumes/Your Disk\""
  exit 1
fi
SRC=
for a in /Install\ macOS*.app; do if [ -d "$a" ]; then SRC="$a"; break; fi; done
if [ -z "$SRC" ]; then echo "This recovery has no installer app: boot the VM's macOS recovery"; exit 1; fi
APP="$T/private/tmp/${SRC##*/}"
echo "== Copying ${SRC##*/} to $T"
mkdir -p "$T/private/tmp"
rm -rf "$APP"
cp -R "$SRC" "$APP"
mkdir -p "$APP/Contents/SharedSupport"
echo "== Copying the installer package from the disc (a few minutes)"
cat "$D"/[Pp][Kk][Gg].[0-9][0-9][0-9] > "$APP/Contents/SharedSupport/SharedSupport.dmg"
if [ -n "$U" ]; then
  echo "== Installing on $T; the VM restarts by itself, and macOS is ready at Setup Assistant"
  if "$APP/Contents/Resources/startosinstall" --agreetolicense --volume "$T"; then
    # In the recovery its restart request goes nowhere: restart once it is prepared.
    sleep 10
    reboot
    exit 0
  fi
  echo "startosinstall failed: opening the installer instead"
fi
echo "== Opening the installer: continue there and choose $N"
for x in InstallAssistant_springboard InstallAssistant; do
  if [ -x "$APP/Contents/MacOS/$x" ]; then exec "$APP/Contents/MacOS/$x"; fi
done
echo "Could not find the installer in $APP"
exit 1
