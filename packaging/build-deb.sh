#!/bin/bash
# Build out/ubuntu-ble-wake-fix_<version>_<arch>.deb (Debian/Ubuntu).
set -euo pipefail
TOP=$(cd "$(dirname "$0")/.." && pwd)
CARGO=${CARGO:-cargo}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$TOP/Cargo.toml" | head -n1)
ARCH=$(dpkg --print-architecture)
cd "$TOP"
"$CARGO" build --release --locked
P=$(mktemp -d)
chmod 755 "$P"
trap 'rm -rf "$P"' EXIT
install -D -m755 target/release/ubuntu-ble-wake-fix "$P/usr/sbin/ubuntu-ble-wake-fix"
install -D -m644 packaging/ubuntu-ble-wake-fix.service "$P/usr/lib/systemd/system/ubuntu-ble-wake-fix.service"
D="$P/usr/share/doc/ubuntu-ble-wake-fix"
install -D -m644 README.md "$D/README.md"
install -D -m644 CHANGELOG.md "$D/CHANGELOG.md"
install -D -m644 docs/how-it-works.md "$D/how-it-works.md"
cat > "$D/copyright" <<EOT
ubuntu-ble-wake-fix
Copyright (C) 2026 Zerun Wang
License: GPL-2.0-or-later (full text: /usr/share/common-licenses/GPL-2)
Source: https://github.com/ZerunW/ubuntu-ble-wake-fix
EOT
chmod 644 "$D/copyright"
install -d "$P/etc/ubuntu-ble-wake-fix"
cat > "$P/etc/ubuntu-ble-wake-fix/exclude" <<'EOT'
# Bluetooth addresses that must NOT wake the system, one per line, e.g.
# C0:FF:EE:00:00:02   # mouse
EOT
mkdir -p "$P/DEBIAN"
chmod 644 "$P/etc/ubuntu-ble-wake-fix/exclude"
echo /etc/ubuntu-ble-wake-fix/exclude > "$P/DEBIAN/conffiles"
install -m755 packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm "$P/DEBIAN/"
SIZE=$(du -sk "$P" | cut -f1)
cat > "$P/DEBIAN/control" <<EOT
Package: ubuntu-ble-wake-fix
Version: $VERSION
Architecture: $ARCH
Maintainer: Zerun Wang <213201277+ZerunW@users.noreply.github.com>
Installed-Size: $SIZE
Depends: libc6, systemd
Recommends: bluez
Section: admin
Priority: optional
Homepage: https://github.com/ZerunW/ubuntu-ble-wake-fix
Description: fix Bluetooth LE keyboards and mice not waking the system
 Keeps the kernel's Remote Wakeup flag set for bonded Bluetooth input
 devices (BlueZ 5.72 on newer kernels never sets it or clears it again)
 and enables USB wakeup for the Bluetooth controller. Works next to any
 BlueZ version without modifying it.
EOT
mkdir -p out
dpkg-deb --root-owner-group --build "$P" "out/ubuntu-ble-wake-fix_${VERSION}_${ARCH}.deb" >/dev/null
echo "out/ubuntu-ble-wake-fix_${VERSION}_${ARCH}.deb"
