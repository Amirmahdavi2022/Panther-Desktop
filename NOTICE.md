# Third-party notices

Panther Desktop is an independent Windows client.

## Relationship to Panther for Android

Panther Desktop shares the engine naming and the Global engine's configuration
with https://github.com/Amirmahdavi2022/Panther-VPN, but none of its code. The
Android app is a fork of https://github.com/hamvex/AetherGUI; this one is not.

Neither is affiliated with, endorsed by, or supported by the upstream projects.
Do not report issues here to them.

## Aether

This application bundles the Aether core from https://github.com/CluvexStudio/Aether,
licensed under GNU AGPL v3.0.

The Windows core is downloaded from the official **v2.0.0** release (the same version the Android build pins) during the
build and verified against the publisher-provided SHA-256 file. The pinned
version lives in `.github/workflows/release.yml`.

Aether and its marks are subject to the upstream project's trademark policy.
Panther is an independent frontend and is not endorsed by CluvexStudio.

## Global engine

The Global engine is the Psiphon tunnel core from
https://github.com/Psiphon-Labs/psiphon-tunnel-core, licensed under GNU GPL v3.0,
which is compatible with the AGPL-3.0 this project is under.

No Windows binary is published for it, so the build compiles `ConsoleClient`
from source at tag **v2.0.40** — the same tag the Android build pins — with the
Go toolchain on the runner.

Psiphon and its marks belong to Psiphon Inc. Panther is an independent client
and is not endorsed by or affiliated with Psiphon Inc.

## Tunnel (sing-box)

The whole-system VPN mode runs sing-box from https://github.com/SagerNet/sing-box,
licensed under GNU GPL v3.0 or later, as a separate program. It embeds the
WireGuard project's Wintun driver.

The Windows build is downloaded from the official **v1.13.21** release and
checked against a SHA-256 pinned in `.github/workflows/release.yml`, since the
release does not publish its own checksum file.

## Country flags

The flags come from flag-icons 7.5.0, https://github.com/lipis/flag-icons,
MIT licence, Copyright (c) 2013 Panayiotis Lipiridis. Only the exit countries
are included, in `app.flags.js`.

## Prowl engine

Not shipped in this build. When it is, it will carry the Xray core from
https://github.com/XTLS/Xray-core, licensed under MPL-2.0, and this notice will
say so.

## Licence

Panther Desktop is distributed under GNU AGPL v3.0.
Complete corresponding source: https://github.com/Amirmahdavi2022/Panther-Desktop
