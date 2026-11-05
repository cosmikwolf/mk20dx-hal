# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `QuadratureDecoder::set_filter` wrote FTMx_FILTER while CH0..CH3 could
  be inputs. FILTER takes effect immediately and must only be written
  while those channels are not in input modes (K20P64M72SF1RM ch36, FTMx_FILTER),
  or a valid edge can be missed. The decoder itself (QUADEN) uses CH0/CH1
  as inputs, and `QuadratureDecoder::new` does not stop other code from
  putting CH2/CH3 (or CH0/CH1) in input capture through `FtmExt::split()`.
  `set_filter` now clears QUADEN and the edge bits of any input-capture
  channel on CH0..CH3 for the write, then restores them.
  
## [0.3.2] - 2026-11-05

### Fixed

- `Adc::start_continuous_scan` (1-2 channels) works. Its DMA read RA for
  both slots, so `results[1]` held channel 0 and RB stayed unread, and
  `auto_disable` stopped the DMA after the first scan, so the results never
  changed. The DMA now reads RA, then RB (`source_offset` 4), goes back to RA
  after each scan, and keeps its request on. A 1-channel scan also turns
  PDB pre-trigger 1 off again, which a 2-channel scan before it left on.

## [0.3.1] - 2026-11-02

### Added

- `UsbBus::with_buffers(usb0, &sim, &clocks, buffers)` and `UsbBuffer`: the
  USB packet buffers come from the application, as many as its endpoints
  need, in place of the HAL's pool for every endpoint in both directions.
  `UsbBus::TAIL_BUFFERS` says how many the BDT block already holds. A MIDI
  device (EP0, EP1 OUT/IN) with `usb-endpoints-4` needs 2: 640 B of USB
  RAM in all, in place of 1,536 B. Measured on flxs1: static RAM -868 B,
  flash +36 B.

### Changed

- USB packet buffers go only to the endpoint directions that a class
  allocates (two each, EP0 four), and ENDPTn enables only those directions
  (as usb_dev.c does with `ENDPOINT_TRANSMIT_ONLY` / `RECEIVE_ONLY`). Before,
  each endpoint number had four buffers and both directions on.
- With `usb-endpoints-4`, the 384 B after the 128-byte BDT, which the
  512-byte alignment of the table left unused, hold 6 packet buffers: USB
  RAM 1,152 B in place of 1,536 B (flxs1 static RAM -364 B with no code
  change). The static keeps the name `BDT`, so a linker script that places
  it by symbol still finds it.
- No 64-bit divide in the HAL: `Delay::delay_ns`, the SPI and UART baud
  calculations and the PIT and LPTMR periods use 32-bit math with the same
  results. A firmware with no other 64-bit divide no longer links
  `__aeabi_uldivmod` (about 930 B): testsuite `delay` -968 B, `spi_baud`
  -1,036 B, `timer` -864 B, `lptmr` -964 B.

## [0.3.0] - 2026-11-02

0.3.0, not 0.2.3: `DmaError` has a new variant (see Added), which breaks an
exhaustive `match` on it.

### Fixed

- `Spi::wait_tx_complete` returned while the last frame still shifted.
  TCF is set by every frame, and the one an earlier frame left set ended
  the wait at once. The callers then flushed the FIFOs, the last frame's
  RX byte landed in the RX FIFO after the flush, and the next byte
  transfer read it: every SPI read after a DMA transfer was one byte late.
  The call overhead between crates mostly hid the race; with LTO it hit on
  every transfer (FLXS1: button reads shifted, flash reads wrong). It now
  clears TCF once the TX FIFO is empty and waits for the last frame's TCF.
  The wait is bounded, for a frame that ended before the clear, which at
  15-30 MHz is common (an RX DMA that took every byte always ends that
  way): two frames at CTAR0's timing, in SR reads (each takes at least one
  bus cycle).
- The blocking `transfer_byte` (`SpiBus` and the byte helpers) and the
  async one drop a byte left in the RX FIFO before the push (MCR
  `CLR_RXF`); the async one also waits for RFDF before it reads POPR (TCF
  can come first).

- `DmaChannel::set_source` now clears the same source from every other
  DMAMUX channel first (CHCFG = 0). The K20 reference manual says two
  CHCFG registers with the same SOURCE give unpredictable behaviour, even
  when one has ENBL clear, and nothing cleared a source after a transfer.
  Found on the FLXS1: SPI0_TX was on for the display (1), DAC (2) and
  flash TX (6) channels, and the flash read on channels 6/7 lost RX bytes,
  so it never completed. Two drivers that share a source must each call
  `set_source` before each transfer and must not run at the same time.
  The scan reads CHCFG as 4 words, about 0.3 us at 120 MHz.
- USB: an OUT endpoint stopped receiving for good when both RX banks
  completed before the class read one (a class that holds off its reads
  for flow control, or two packets in one interrupt). `poll()` kept only
  the last bank, so `read()` re-armed it and left the other bank owned
  by the CPU; the controller then NAKed every later packet. `poll()` now
  keeps the second bank behind the first, and `read()` hands it out next.

### Added

- Feature `usb-endpoints-4`: the USB bus has endpoints 0-3 in place of
  0-15, and its buffer pool is 1 KB in place of 4 KB (four 64-byte buffers
  per endpoint). Enough for EP0 and three more endpoint numbers, for
  example a USB-MIDI device (EP0, one bulk OUT, one bulk IN). Default
  unchanged.
- `DmaReadTransfer::wait_timeout(max_cycles)` and
  `DmaTransfer::wait_timeout(max_cycles)`: the same checks as `wait()`,
  then a limit in DWT cycles. On the limit they return
  `Err(DmaError::Timeout)` and the Drop impl stops the channels. `wait()`
  loops with no limit, so an RX channel that misses one byte never returns.
  The caller must have the DWT cycle counter running. Added for the FLXS1
  flash reads.
- `DmaError::Timeout`. `DmaError` is not `#[non_exhaustive]`, so an
  exhaustive `match` on it outside the crate needs the new arm.
- `spi::Baud`: the CTAR0 baud fields (BR, PBR, DBR) for one SCK rate,
  from `calc_baud` (`Baud::new(&clocks, hz)`, `Baud::from_bus_clk`,
  `Baud::rate`).
- `Spi::baud` and `Spi::set_baud`: read, or change, the SCK rate of CTAR0
  between transactions, for devices on one bus that need different rates.
  `set_baud` halts the module to the Stopped state for the write (K20 RM
  45.4.1) and runs it again; the same rate again is one register read.
- `spi::frame_bus_cycles`: the bus cycles of one frame for given CTAR
  fields (bits, SCK period, the three delays).

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

[0.3.2]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.3.2
[0.3.1]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.3.1
[0.3.0]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.3.0
[0.2.2]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.2
[0.2.1]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.1
[0.2.0]: https://github.com/cosmikwolf/mk20dx-hal/releases/tag/v0.2.0
