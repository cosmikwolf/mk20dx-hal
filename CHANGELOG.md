# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-09-14

Working USB, plus a dependency refresh. USB MIDI now enumerates; before this
release it never did. Every breaking dependency item below is a major bump
that reaches the public API, so downstream crates must move in lockstep.

### Fixed — USB now enumerates

USB MIDI did not enumerate at all before this release. Seven defects in
`src/usb.rs`, verified on hardware (MK20DX256 at a 120 MHz PLL, macOS host),
which now enumerates as a USB MIDI device and reaches `Configured`:

- `enable()` never set `CONTROL[DPPULLUPNONOTG]`, so no host ever saw an attach.
- The USB clock divider was wrong at 120 MHz.
- `suspend()` asserted `USBCTRL[SUSP]`, which makes the transceiver ignore
  everything but resume signalling, so the host's bus reset was never seen.
  It is now a no-op.
- `poll()` checked SLEEP/RESUME *before* servicing tokens. Because a SETUP
  freezes the controller until firmware clears `CTL[TXSUSPENDTOKENBUSY]` in
  the token loop, this livelocked: frozen, so idle, so SLEEP, so return early,
  so still frozen. Tokens are now serviced first. `ISTAT[SOFTOK]` is cleared.
- `CTL` was read-modify-written. `TXSUSPENDTOKENBUSY` reads as 1 while frozen,
  so `modify()` re-asserted the freeze every time. `CTL` is now always written
  whole, as the reference driver does.
- Only the EVEN RX bank was armed and `read()` re-armed a separately tracked
  bank, desynchronising from the controller's ping-pong after the first
  packet. Both banks are armed now, and `read()` gives back the bank that
  just completed.
- A SETUP left stale TX descriptors armed, so an IN from an abandoned
  transfer could answer the next IN token. Both EP0 TX descriptors are
  dropped on SETUP.

Also added to match the reference init order: the `USBTRC0` module reset and
the undocumented `USBTRC0 = 0x40` bit, before `CTL` is enabled.

### Breaking

- **`fugit` 0.3 -> 0.6.** `time::Hertz`, `time::KiloHertz` and `time::MegaHertz` are
  re-exports of `fugit` rate types, and `pit::PitChannel::start()` takes a
  `fugit::MicrosDurationU32`. A downstream crate that names any `fugit` type must move
  to `fugit = "0.6"` in the same change, or Cargo links two incompatible copies and
  reports `expected Duration<u32, 1, 1000000>, found Duration<u32, 1, 1000000>`.

  Renames inside this crate (relevant if you call the same methods):
  - `Rate::raw()` -> `Rate::to_raw()`
  - `Duration::ticks()` -> `Duration::as_ticks()`
  - `Duration::millis(x)` / `secs(x)` -> `from_millis(x)` / `from_secs(x)`

  The `ExtU32` shorthands (`600u32.micros()`) and `time::U32Ext` (`1_000u32.Hz()`) are
  unchanged.

- **`defmt` 0.3 -> 1** (the `defmt` feature). `defmt 0.3.100` was already a shim that
  re-exported `defmt 1.0`, so this is a version-requirement change with no API change.
  It removes the duplicate shim crate from downstream lockfiles.

- **`embassy-sync` 0.7 -> 0.8** (the `async` feature). Used internally for
  `waitqueue::AtomicWaker`, which 0.8.0 does not change. Declared here because a
  downstream crate that also depends on `embassy-sync` must match. Pulls in
  `embedded-io-async` 0.7, removing a duplicate.

- **`UsbBusExt::usb_bus()` takes `&Clocks`:** `usb0.usb_bus(&sim)` becomes
  `usb0.usb_bus(&sim, &clocks)`. The USB divider was hard-coded, so anything
  other than a 120 MHz PLL clocked USB at the wrong rate and silently failed
  to enumerate — a stock 72 MHz Teensy 3.1/3.2 got 28.8 MHz. It is now derived
  from the configured PLL (72, 96 or 120 MHz supported; anything else panics
  at init rather than failing silently).

- **MSRV is now 1.82** (was 1.81). `fugit` 0.6 uses const floating-point arithmetic,
  stabilized in Rust 1.82. `fugit` declares no `rust-version`, so Cargo does not catch
  this: a 1.81 build fails with `E0658: floating point arithmetic is not allowed in
  constant functions` pointing inside `fugit`.

### Changed

- `embedded-io` 0.6 -> 0.7 and `embedded-io-async` 0.6 -> 0.7 unified across the tree.
- Minimum `cortex-m` 0.7.9 and `cortex-m-rt` 0.7.6 in the lockfile.

### Notes

- Outside `src/usb.rs`, no logic changes: source edits were limited to the
  mechanical `fugit` renames.
- Verified on Teensy 3.2 hardware: 28 test binaries, 24 passing, with the same four
  pre-existing failures as before the bump. `usb` now passes 6/6, leaving
  `dac_cmp_analog`, `gpio` and `i2c`.

## [0.1.1]

- Documented that `mk20d5` (Teensy 3.0) is untested on hardware.
- Manifest points at the repository; fixed the docs rendered on crates.io.

## [0.1.0]

- Initial release.

[0.2.0]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.0
