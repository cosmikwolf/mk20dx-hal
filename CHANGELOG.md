# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.2] - 2026-10-15

### Fixed

- `start_multi_channel_scan` (3+ channels): the channel-to-slot map moved
  one step every scan, so `read_latest(i)` did not always give
  `channels[i]`. DMA-A linked DMA-B on the minor loop only, and the eDMA
  does not run that link on the minor loop that ends the major loop (K20 RM
  ch21, `CITER[ELINK]`). DMA-B wrote SC1A N-1 times per N results. DMA-A now
  links DMA-B on the minor and the major loop (`ChannelLink::Both`). Found
  on the FLXS1 (ADC1, 4 channels, PDB 1 kHz, DMA 4/5).
- `start_multi_channel_scan` and `start_continuous_scan` ignored
  `ScanConfig::modulus`. They called `Pdb::load_ok()` before
  `Pdb::enable()`, and LDOK works only while PDBEN is set (K20 RM ch35), so
  MOD kept its reset value 0xFFFF: every scan ran at bus clock / 65536 (549
  Hz at 72 MHz, 916 Hz at 120 MHz). The PDB is now enabled before LDOK. A
  scan now runs at the configured rate; check timing that used the old rate.
- Both scan functions wrote `SC3 = 0` and so dropped the hardware
  averaging that `Adc::set_averaging()` set. They now keep `AVGE`/`AVGS`
  and clear only `ADCO`/`CAL`. Call `set_averaging()` after `calibrate()`
  and before the scan. `ScanConfig` does not change.
- A PDB sequence error (a trigger while a conversion runs, for example
  averaging that is longer than the PDB period) stopped that scan and every
  later one: `CHnS[ERR]` blocks the pre-trigger and a PDB disable does not
  clear it. A scan start now clears it. The overrun itself still stops the
  running scan; the `ScanConfig` docs give the timing rule.
- `start_multi_channel_scan` docs said that the caller fills `mux_buf`. The
  function fills it.

### Known issue

- `start_continuous_scan` (1-2 channels) reads the RA result register for
  both slots and stops after the first scan. Use `start_multi_channel_scan`
  (3+ channels; repeat a channel to fill a slot).
- An 8-byte EP0 does not enumerate on macOS (see 0.2.1).

## [0.2.1] - 2026-10-14

### Added — interrupt-driven USB

- `UsbBus::enable_sof_interrupt(bool)`: the 1 ms start-of-frame token as a
  USB0 interrupt source. Off by default; it survives a bus reset.
- `UsbBus::take_sof() -> bool`: true once after a `poll()` that saw a SOF.
- `UsbBus::frame_number() -> u16`: the 11-bit frame number.
- Module docs: an RTIC example with `poll()` in a `binds = USB0` task.
  Both module doc examples now set `max_packet_size_0(64)`; see Known issue.

### Fixed

- Bulk/interrupt OUT dropped every second packet. Both RX banks were armed
  for DATA0, so the DATA1 packet that lands in the ODD bank was discarded by
  the toggle check. EVEN now expects DATA0 and ODD expects DATA1, as in
  Teensy `usb_dev.c`; EP0 re-arms with DATA1 after each packet, as there.
  Verified on hardware: 4 of 4 Note On packets arrive, 3 of 4 before.
- RESUMEEN was on at all times. K20 RM ISTAT: "When not in suspend mode this
  interrupt must be disabled." It is now on only between `suspend()` and
  `resume()`/`reset()`, and `poll()` reports `Resume` only in that window.

### Known issue

- With `max_packet_size_0(8)` (the usb-device default) a macOS host stops
  after SET_ADDRESS and the device never reaches `Configured`. Use
  `max_packet_size_0(64)`. Not yet investigated.

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

[0.2.2]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.2
[0.2.1]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.1
[0.2.0]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.0
