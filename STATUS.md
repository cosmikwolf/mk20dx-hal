# mk20dx-hal: Project Status

**Last updated:** 2026-03-01 (ADC multi-channel DMA scan, DMA channel linking)

---

## Target Hardware

| Feature | MK20D5 (Teensy 3.0) | MK20D7 (Teensy 3.1/3.2) |
|---------|---------------------|-------------------------|
| Chip | MK20DX128VLH5 | MK20DX256VLH7 |
| Core | Cortex-M4 (no FPU) | Cortex-M4 (no FPU) |
| Max Clock | 48 MHz | 72 MHz |
| Flash | 128 KB | 256 KB |
| RAM | 16 KB | 64 KB |
| DMA Channels | 4 | 16 |
| UART | 3 (UART0-2) | 3 (UART0-2) |
| SPI | 1 (SPI0) | 2 (SPI0-1) |
| I2C | 1 (I2C0) | 2 (I2C0-1) |
| FTM | 2 (FTM0-1) | 3 (FTM0-2) |
| ADC | 1 (ADC0) | 2 (ADC0-1) |
| USB | 1 (USB0) | 1 (USB0) |
| DAC | 0 | 1 (DAC0) |

---

## PAC Dependency Status

The `mk20dx-pac` crates are mature (Phase 5 complete, Phase 6 publishing in progress). See `../mk20dx-pac/docs/STATUS.md` for details.

| PAC Crate | Compiles | Correctness Patches | Ergonomics Patches | Validation |
|-----------|----------|--------------------|--------------------|------------|
| `mk20d5` | OK (0 errors) | 2 applied (SIM, FMC) | 19 applied (DMAMUX enums, DMA TCD clusters, 17 common semantic enum patches) | 44 peripherals verified, 0 address mismatches |
| `mk20d7` | OK (0 errors) | 0 needed (clean SVD) | 25 applied (DMAMUX enums, DMA TCD clusters, 17 common semantic enum patches, 6 variant-specific) | 48 peripherals verified, 0 address mismatches |

Semantic enum patches now provide named enums across PORT (MUX + bitfields), FTM, ADC, MCG, SIM SOPT2, SIM SCGC, DMA ATTR, SPI, and UART fields — the HAL uses these directly instead of raw bit patterns.

PAC is dual-licensed MIT/Apache-2.0, has README.md, and Cargo.toml metadata is ready for publishing.

---

## Phase 1: Project Scaffold — COMPLETE

- [x] `Cargo.toml` with dependencies and feature flags
- [x] `src/lib.rs` with PAC re-export and feature gates
- [x] `build.rs` for linker script selection
- [x] `memory.x` for both variants (in `memory/` directory)
- [x] `.cargo/config.toml` with default target
- [x] Compiles with `cargo check` for both variants (zero errors, zero HAL warnings)

---

## Phase 1.5: Flash Configuration & Watchdog — COMPLETE

- [x] Flash configuration field at 0x400 (`src/flash_config.rs`)
- [x] Watchdog disable consuming WDOG peripheral (`src/watchdog.rs`)
- [x] `WdogExt` extension trait on `pac::Wdog` with `disable()` method

---

## Phase 2: Clock Configuration — COMPLETE

- [x] SIM clock divider configuration (OUTDIV1/2/4)
- [x] MCG configuration (FEI → FBE → PBE → PEE mode transitions)
- [x] OSC crystal oscillator enable (16 MHz, SC8P+SC2P load caps)
- [x] `Clocks` token struct (core_clk, bus_clk, flash_clk frequencies)
- [x] Feature-gated PLL math: mk20d7=72 MHz, mk20d5=48 MHz
- [x] `McgExt` extension trait on `pac::Mcg` with `constrain()` → `Mcg` → `freeze(osc, &sim)` → `Clocks`
- [x] `ClockSpeed` enum (mk20d7 only): `Mhz72`, `Mhz96`, `Mhz120` overclock presets
- [x] `Mcg::freeze_at(speed, osc, &sim)` — parameterized clock configuration (mk20d7 only)
- [x] `freeze()` delegates to `freeze_at(ClockSpeed::Mhz72, ..)` on mk20d7
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/clocks.rs` (5 tests, 72 MHz)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/clocks_96mhz.rs` (5 tests, 96 MHz overclock)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/clocks_120mhz.rs` (5 tests, 120 MHz overclock)

### Clock Speed Presets (mk20d7)

| Speed | Core | Bus | Flash | PRDIV | VDIV | Notes |
|-------|------|-----|-------|-------|------|-------|
| `Mhz72` | 72 MHz | 36 MHz | 24 MHz | ÷8 | ×36 | Datasheet rated max (default) |
| `Mhz96` | 96 MHz | 48 MHz | 24 MHz | ÷4 | ×24 | Overclock, widely used |
| `Mhz120` | 120 MHz | 60 MHz | 24 MHz | ÷4 | ×30 | Overclock, Teensyduino default |

**Known limitation:** USB clock divider (SIM CLKDIV2) is currently hardcoded for 72 MHz PLL output. USB will not enumerate correctly at 96 or 120 MHz until the USB driver is updated to derive the correct divider from the actual PLL frequency.

Reference: K20 ref manual chapters 5 (Clock Distribution), 12 (SIM), 24 (MCG)

---

## Phase 2.5: SysTick Delay — COMPLETE

- [x] `Delay` struct wrapping SysTick (`src/delay.rs`)
- [x] `embedded_hal::delay::DelayNs` implementation
- [x] Loop handling for delays exceeding 24-bit SysTick reload max
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/delay.rs` (7 tests, PIT crosscheck)

---

## Phase 3: GPIO — COMPLETE

- [x] `Pin<PORT, N, MODE>` type with const generics
- [x] Type-state mode types: `Input<Floating/PullUp/PullDown>`, `Output<PushPull/OpenDrain>`, `Alternate<MUX>`, `Disabled`
- [x] Port splitting via `gpio_port_impl!` macro → `PortXPins` structs
- [x] PORT PCR mux configuration for GPIO mode
- [x] PORT PCR pull-up/pull-down/open-drain configuration
- [x] `embedded_hal::digital::InputPin` (reads PDIR)
- [x] `embedded_hal::digital::OutputPin` (writes PSOR/PCOR — atomic, no RMW)
- [x] `embedded_hal::digital::StatefulOutputPin` (reads PDOR, toggle via PTOR)
- [x] All 5 ports (A-E) with 32 pins each
- [x] SIM SCGC5 clock gating in port `new()`
- [x] `GpioExt` extension trait on `pac::Porta`/etc. with `split(gpio, &sim)` → `PortXPins`
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/gpio.rs` (8 tests) + `gpio_loopback.rs` (2 tests, requires PTD5→PTD6 wire)

Reference: K20 ref manual chapters 11 (PORT), 43 (GPIO)

---

## Phase 4: UART / Serial — COMPLETE

- [x] `Serial<UART>` type wrapping PAC UART peripheral
- [x] Pin assignment via alternate function (MUX=3 for UART pins)
- [x] Baud rate calculation from clock frequencies
- [x] UART0 FIFO support (8-byte TX/RX FIFO)
- [x] `embedded_hal_nb::serial::Read<u8>`
- [x] `embedded_hal_nb::serial::Write<u8>`
- [x] `embedded_io::Read`
- [x] `embedded_io::Write` with `flush()`
- [x] `UartExt` extension trait on `pac::Uart0`/`Uart1`/`Uart2`
- [x] Split into `Tx<UART>` and `Rx<UART>` halves
- [x] DMA helpers: `disable_dma_requests()`, `wait_tx_complete()`
- [x] Semantic PAC enum names (UART C1-C5, S1, S2, PFIFO, CFIFO, SFIFO)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/uart_loopback.rs` (5 tests, requires PTD3→PTD2 wire)

Reference: K20 ref manual chapter 35 (UART)

---

## Phase 5: SPI — COMPLETE

- [x] `Spi<SPI>` type wrapping PAC SPI (DSPI) peripheral
- [x] Pin assignment (SCK, MOSI, MISO via alternate function MUX=2)
- [x] CTAR configuration (baud rate, frame size, polarity, phase)
- [x] Baud rate calculation (PBR × BR × DBR search for closest ≤ target)
- [x] `embedded_hal::spi::SpiBus` (read, write, transfer, transfer_in_place, flush)
- [x] `SpiExt` extension trait on `pac::Spi0` (+ `pac::Spi1` for mk20d7)
- [x] SPI1 feature-gated behind `mk20d7`
- [x] Macro-generated implementation for each SPI instance
- [x] `pack_pushr()` const fn — build 32-bit PUSHR command words (data + PCS + CONT + EOQ)
- [x] MCR control: `flush_fifos()`, `set_rx_fifo()`, `set_rooe()`
- [x] Status/request control: `clear_status()`, `disable_dma_requests()`, `wait_tx_complete()`
- [x] `write_dma_pushr()` — 32-bit DMA writes to PUSHR with full command field control
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/spi_loopback.rs` (7 tests, requires PTC6→PTC7 wire)

Reference: K20 ref manual chapter 37 (DSPI)

---

## Phase 6: I2C — COMPLETE

- [x] `I2c<I2C>` type wrapping PAC I2C peripheral
- [x] Sealed trait instance abstraction (I2C0 + I2C1 with shared RegisterBlock)
- [x] Pin assignment (SDA, SCL via alternate function MUX=2)
- [x] Clock rate configuration via ICR divider table (64-entry MULT×ICR search)
- [x] `embedded_hal::i2c::I2c<SevenBitAddress>` (`transaction` with default `read`/`write`/`write_read`)
- [x] Correct ACK/NACK sequencing for multi-byte reads
- [x] Consecutive same-direction operation grouping per trait contract
- [x] `I2cExt` extension trait on `pac::I2c0` (+ `pac::I2c1` for mk20d7)
- [x] I2C1 feature-gated behind `mk20d7`
- [x] Distinct error types: `ArbitrationLoss`, `AddressNack`, `DataNack`
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/i2c.rs` (4 tests, requires 4.7kΩ pull-ups on PTB0/PTB1)

Reference: K20 ref manual chapter 38 (I2C)

---

## Phase 7: PIT Timer — COMPLETE

- [x] `PitChannel<CH>` type with const generic channel index (4 channels)
- [x] `PitExt` extension trait on `pac::Pit` with `split(sim, clocks)` → `PitChannels`
- [x] SIM SCGC6 clock gating, module enable, debug freeze
- [x] `start()` with `fugit::MicrosDurationU32` period (bus_clk conversion)
- [x] `start_ticks()` for raw tick count (LDVAL)
- [x] `wait()` → `nb::Result` polling for TIF (w1c clear)
- [x] `cancel()` to stop timer
- [x] `current()` to read down-counter value (CVAL)
- [x] `enable_interrupt()` / `disable_interrupt()` / `has_expired()` / `clear_interrupt()`
- [x] No `#[cfg]` needed — both mk20d5 and mk20d7 have identical PIT
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/timer.rs` (10 tests)

Reference: K20 ref manual chapter 28 (PIT)

---

## Phase 8: PWM — COMPLETE

- [x] `PwmChannel<FTM, CH>` type with const generic channel index
- [x] `FtmExt` extension trait on `pac::Ftm0`/`Ftm1`/`Ftm2` with `pwm(freq, clocks, sim)` → channel set
- [x] FTM prescaler auto-selection and period (MOD register) configuration
- [x] Per-channel duty cycle (CnV register) with `enable()`/`disable()`
- [x] `embedded_hal::pwm::SetDutyCycle` (infallible error type)
- [x] Channel sets: `Ftm0Channels` (8 ch), `Ftm1Channels` (2 ch), `Ftm2Channels` (2 ch, mk20d7 only)
- [x] Macro-generated implementation for each FTM instance
- [x] FTM2 feature-gated behind `mk20d7`
- [x] Edge-aligned PWM, high-true (MSB:MSA=10, ELSB:ELSA=10)
- [x] Write protection disabled during init
- [x] Combined mode: `FtmChannelPair<FTM, PAIR>` with complementary output, dead-time, and inversion
- [x] `into_combined(partner)` on even channels, `into_channels()` to release pair
- [x] Dead-time configuration: `FtmTimer::set_deadtime(prescaler, value)` (timer-wide DEADTIME register)
- [x] Complementary output: `enable_complementary()`/`disable_complementary()` (COMP bit per-pair)
- [x] Dead-time enable: `enable_deadtime()`/`disable_deadtime()` (DTEN bit per-pair)
- [x] Output inversion: `set_inversion()` via INVCTRL (double-buffered)
- [x] Enhanced sync: SYNCONF.SYNCMODE=1 set during `split()`/`pwm()` (backward-compatible)
- [x] PWM sync control: `software_sync()`, `set_sync_loading_points(at_min, at_max)` on FtmTimer
- [x] `ftm_pair_impl!` macro: FTM0 (4 pairs), FTM1 (1 pair), FTM2 (1 pair, mk20d7 only)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/pwm.rs` (7 tests, register-only)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/pwm_combined.rs` (18 tests, register-only)

Reference: K20 ref manual chapter 36 (FTM), §36.4.15 (COMBINE), §36.4.16 (DEADTIME), §36.4.21 (SYNC), §36.4.22 (INVCTRL), §36.4.27 (SYNCONF)

---

## Phase 9: ADC — COMPLETE

- [x] `Adc<ADC>` type wrapping PAC ADC peripheral
- [x] `AdcExt` extension trait on `pac::Adc0` (+ `pac::Adc1` for mk20d7) with `adc(clocks, sim)`
- [x] Calibration sequence (16-bit, 32-sample averaging, PG/MG gain calculation)
- [x] Resolution selection (`set_resolution`: 8/10/12/16-bit via `Resolution` enum)
- [x] Hardware averaging configuration (`set_averaging`: disabled/4/8/16/32 via `Averaging` enum)
- [x] Blocking single-shot conversion API (`read(channel)` → `u16`)
- [x] Default config: 10-bit, bus clock /2, ADIV /4, long sample, software trigger
- [x] Macro-generated implementation for each ADC instance
- [x] ADC1 feature-gated behind `mk20d7`
- [x] SC3 w1c hazard avoided (write() instead of modify())
- [x] PDB-triggered continuous scanning (1-2 channels via `start_continuous_scan()`)
  - [x] `ScanConfig` struct (channels, modulus, prescaler, multiplier)
  - [x] `ContinuousScan` handle with `read_latest()`, `stop()`, abort-on-drop
  - [x] PDB pre-trigger 0 + back-to-back pre-trigger 1 for 2-channel mode
  - [x] DMA reads ADC RA into results buffer, auto-wraps via `dest_last_adjust`
- [x] Multi-channel scanning (3+ channels via `start_multi_channel_scan()`)
  - [x] `MultiChannelScan` handle with `read_latest()`, `stop()`, abort-on-drop
  - [x] DMA minor loop channel linking: DMA-A reads results, links to DMA-B
  - [x] DMA-B writes rotated mux buffer entries to SC1A (channel cycling)
  - [x] PDB fires single pre-trigger 0 continuously; DMA handles mux rotation
  - [x] `sc1a_dma_addr()` helper for DMA mux writes to SC1[0]
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/adc.rs` (10 tests, internal references)

Reference: K20 ref manual chapter 31 (ADC), chapter 34 (PDB)

---

## Phase 10: DMA — COMPLETE

- [x] `DmaChannel<const CH: u8>` zero-sized per-channel handle (4 on mk20d5, 16 on mk20d7)
- [x] `DmaChannels` struct returned by `DmaExt::split()`
- [x] `DmaExt` extension trait on `pac::Dma` consuming DMA + DMAMUX peripherals
- [x] `TransferSize` enum (Bits8/Bits16/Bits32/Burst16) mapped to TCD ATTR SSIZE/DSIZE
- [x] `TransferConfig` struct for full TCD configuration
- [x] `DmaSource` newtype with 40+ named constants for DMAMUX routing
- [x] `DmaError` enum (9 error variants parsed from ES register)
- [x] DMAMUX source routing: `set_source()`, `disable_source()`
- [x] TCD configuration: `unsafe configure()` writes all TCD registers
- [x] Transfer control: `enable_request()`, `disable_request()`, `start()`
- [x] Status: `is_complete()`, `has_error()`, `is_active()`, `error_status()`
- [x] Flag management: `clear_done()`, `clear_interrupt()`, `clear_error()`
- [x] Interrupts: `enable_interrupt()`, `disable_interrupt()`, `enable_error_interrupt()`, `disable_error_interrupt()`
- [x] Circular buffers: `dest_modulo` (DMOD field) for hardware address wrapping at 2^N byte boundaries
- [x] Auto-disable control: `auto_disable` flag (DREQ bit) for continuous/circular transfers
- [x] TCD readback: `current_dest_addr()` for tracking DMA write position
- [x] Convenience: `configure_memcpy()`, `configure_peripheral_read()`, `configure_peripheral_write()`
- [x] DCHPRI byte-swapped index mapping (`ch ^ 3`)
- [x] Default init: stall in debug, fixed priority, channel N = priority N
- [x] mk20d7-only DMA sources feature-gated (SPI1, I2C1, FTM2, ADC1, CMP2)
- [x] Channel linking: `configure_linked()` with `ChannelLink` enum (MinorLoop, MajorLoop, Both)
- [x] Scatter-gather: `ScatterGatherTcd` (32-byte aligned) with `configure_scatter_gather()`, `from_config()`, `set_next()`
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/dma.rs` (11 tests, memory-to-memory)

Reference: K20 ref manual chapter 21 (eDMA), chapter 22 (DMAMUX)

---

## Phase 11: USB Device — COMPLETE

- [x] `UsbBus` struct implementing `usb_device::bus::UsbBus` trait (v0.3)
- [x] `UsbBusExt` extension trait on `pac::Usb0` with `usb_bus(sim)` → `UsbBus`
- [x] BDT (Buffer Descriptor Table) — 512-byte aligned static, 16 endpoints × 2 dir × 2 ping-pong
- [x] Static endpoint buffer pool — 64 × 64-byte buffers, 4-byte aligned
- [x] USB 48 MHz clock configuration (SIM SOPT2/CLKDIV2/SCGC4)
  - mk20d7: 72 MHz PLL × 2/3 = 48 MHz (USBFRAC=1, USBDIV=2)
  - mk20d5: 48 MHz PLL × 1/1 = 48 MHz (USBFRAC=0, USBDIV=0)
- [x] Endpoint allocation with type/direction tracking
- [x] Ping-pong buffer management (EVEN/ODD banks)
- [x] DATA0/DATA1 toggle tracking per endpoint per direction
- [x] TOKDNE FIFO processing (read STAT before clearing ISTAT)
- [x] SETUP token detection and data toggle reset
- [x] Stall/unstall with data toggle reset
- [x] Suspend/resume via USBCTRL SUSP bit
- [x] Force reset via USBENSOFEN toggle
- [x] ISTAT/ERRSTAT w1c correct handling (write() not modify())
- [x] No `#[cfg]` needed — both mk20d5 and mk20d7 have identical USB0
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/usb.rs` (6 tests, init/alloc only)

Reference: K20 ref manual chapter 34 (USB OTG / USB-FS)

---

## Phase 12: Hardware Validation Test Suite — COMPLETE (blocking code), PENDING (async + on-target execution)

### Blocking Tests (Complete)

- [x] Test framework selection: `defmt-test` v0.3 with `probe-rs` runner
- [x] Infrastructure: Cargo.toml, .cargo/config.toml, build.rs updated
- [x] 12 self-test binaries (no external wiring): watchdog (3), clocks (5), clocks_96mhz (5), clocks_120mhz (5), gpio (8), delay (7), timer (10), adc (10), dma (11), pwm (7), i2c (4), usb (6)
- [x] 11 additional self-test binaries (Phases 13-21): crc (7), dac (6), flash (6), eeprom (5), lptmr (8), rtc (8), cmp (7), power (3), llwu (5), pwm_advanced (9), pwm_combined (18)
- [x] 3 loopback test binaries (require wiring): gpio_loopback (2), uart_loopback (5), spi_loopback (7)
- [x] All 26 blocking test binaries compile cleanly (`cargo check --tests`)
- [x] Makefile with `make self-tests`, `make loopback-tests`, `make all-tests`, `make check`
- [ ] On-target execution of self-tests
- [ ] On-target execution of loopback tests (with wiring)

### Async Tests (Planned)

- [ ] 2 async self-test binaries (no wiring): async_timer (6), async_dma (5)
- [ ] 4 async loopback test binaries (require wiring): async_gpio_loopback (5), async_uart_loopback (5), async_spi_loopback (7), async_i2c (4)
- [ ] Minimal `block_on()` executor helper (SEV/WFE-based)
- [ ] Interrupt handler wiring + NVIC unmask in each test binary
- [ ] All 6 async test binaries compile cleanly
- [ ] On-target execution of async self-tests
- [ ] On-target execution of async loopback tests

**Total: 203 tests across 32 binaries (171 blocking + 32 async)**

### Tests Requiring Additional Hardware (Not Implemented)

| Category | Hardware Needed |
|----------|----------------|
| I2C device communication | I2C EEPROM (AT24C32) + 4.7kΩ pull-ups |
| USB enumeration / data transfer | USB cable + host PC |
| PWM frequency/duty cycle accuracy | Oscilloscope or logic analyzer |
| Delay absolute accuracy | Oscilloscope or frequency counter |
| ADC external pin accuracy | Known voltage source + wiring |
| UART baud rate accuracy | Logic analyzer |
| DMA peripheral transfers | Peripheral-specific wiring |
| SPI with real device | SPI flash or EEPROM |

See STRATEGY.md Phase 12.4–12.6 for full details.

---

## Phase 13: Flash Memory (FTFL) — COMPLETE

- [x] `Flash` struct wrapping consumed PAC `Ftfl` peripheral (`src/flash.rs`)
- [x] `FlashExt` extension trait on `pac::Ftfl` with `flash()` → `Flash`
- [x] RAM trampoline (`launch_command`) via `#[link_section = ".data"]` for single-block flash safety
- [x] `erase_sector(address)` — Erase Sector command (0x09), 2 KB sectors
- [x] `program_longword(address, data)` — Program Longword command (0x06), 4-byte aligned
- [x] `is_protected(address)` — FPROT register check (32-bit, per-region)
- [x] `security_status()` — FSEC register read
- [x] Safety floor at 0x800 (sector 0 contains vector table + flash config field)
- [x] `embedded_storage::nor_flash::ReadNorFlash` (memory-mapped read, READ_SIZE=1)
- [x] `embedded_storage::nor_flash::NorFlash` (WRITE_SIZE=4, ERASE_SIZE=2048)
- [x] `FlashError` enum with `NorFlashError` impl (NotAligned, OutOfBounds, Protected, AccessError, ProtectionViolation, CommandFailure)
- [x] Critical section (`cortex_m::interrupt::free`) around flash commands
- [x] No `#[cfg]` needed for driver logic — both mk20d5 and mk20d7 have identical FTFL (only capacity differs)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/flash.rs` (6 tests, read-only + error path validation — NO erase/write tests due to bricking risk)

Reference: K20 ref manual chapter 29 (FTFL)

---

## Phase 14: DAC — COMPLETE

- [x] `Dac` struct wrapping consumed PAC `Dac0` peripheral (`src/dac.rs`)
- [x] `DacExt` extension trait on `pac::Dac0` with `dac(sim)` → `Dac`
- [x] 12-bit output via DAT0L + DAT0H registers (`set_value`, `get_value`)
- [x] Voltage reference selection (`set_vref`: Vref1/Vref2)
- [x] Enable/disable control
- [x] Init defaults: DACEN=1, VREF1 (VDDA), software trigger, buffer disabled, high power, output=0
- [x] Entire module `#[cfg(feature = "mk20d7")]` — DAC0 not present on mk20d5
- [x] No `Clocks` parameter needed
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/dac.rs` (6 tests, register-only + value roundtrip)

Reference: K20 ref manual chapter 33 (DAC)

---

## Phase 14: RTC — COMPLETE

- [x] `Rtc` struct wrapping consumed PAC `Rtc` peripheral (`src/rtc.rs`)
- [x] `RtcExt` extension trait on `pac::Rtc` with `rtc(sim)` → `Rtc`
- [x] Seconds counter read/write (`seconds`, `set_time`)
- [x] `TimeInvalid` error type for reading uninitialized time
- [x] TCE constraint: `set_time()` disables counter, writes TSR, re-enables
- [x] Alarm support (`set_alarm`, `alarm_fired`, `clear_alarm`)
- [x] Interrupt enable/disable for alarm and seconds
- [x] Enable/disable counter control
- [x] Init: clock gate, clear SWR, enable oscillator (OSCE=1, SC8P+SC2P ~10pF), disable interrupts, start counter if valid
- [x] No `Clocks` parameter (RTC uses independent 32.768 kHz oscillator)
- [x] No `#[cfg]` needed — both mk20d5 and mk20d7 have identical RTC
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/rtc.rs` (8 tests, uses 32.768 kHz oscillator)

Reference: K20 ref manual chapter 23 (RTC)

---

## Phase 14: CMP — COMPLETE

- [x] `Cmp<INST>` generic driver with `PhantomData` instance markers (`src/cmp.rs`)
- [x] `CmpExt` extension trait on `pac::Cmp0`/`Cmp1`/`Cmp2` with `cmp(plus, minus, sim)` → `Cmp<Instance>`
- [x] `cmp_impl!` macro for per-instance implementation (follows `adc.rs` pattern)
- [x] Input selection (`Input` struct with IN0-IN7 + INTERNAL_DAC alias)
- [x] Comparator output read (`output()`)
- [x] Input mux control (`set_plus_input`, `set_minus_input`)
- [x] Internal 6-bit DAC (`set_internal_dac`, `disable_internal_dac`)
- [x] Hysteresis control (`set_hysteresis`: Level0-Level3)
- [x] Output inversion (`set_inverted`)
- [x] Enable/disable control
- [x] Edge detection (`rising_edge`, `falling_edge`, `clear_flags`)
- [x] Interrupt control (`enable_rising_interrupt`, `enable_falling_interrupt`, `disable_interrupts`)
- [x] SCR w1c hazard handled: all SCR writes use `write()` with manual config bit preservation
- [x] CMP0 + CMP1 on both variants, CMP2 feature-gated behind `mk20d7`
- [x] Shared clock gate (SIM SCGC4 CMP bit)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/cmp.rs` (7 tests, register-level, internal DAC self-referencing)

Reference: K20 ref manual chapter 32 (CMP)

---

## Phase 15: Async Support — COMPLETE

All async code is behind `#[cfg(feature = "async")]` and requires the `async` Cargo feature flag. The HAL exports `on_*_interrupt()` functions; users wire these to `#[interrupt]` handlers and unmask IRQs in the NVIC. Uses `embassy-sync` `AtomicWaker` for interrupt-to-task waking. Executor-agnostic (works with Embassy, RTIC, or any `core::task::Waker`-based executor).

### Dependencies Added (optional, gated by `async` feature)

| Crate | Version | Purpose |
|-------|---------|---------|
| `embassy-sync` | 0.7 | `AtomicWaker` for interrupt-driven waking |
| `embedded-hal-async` | 1.0 | Async trait definitions (delay, digital, SPI, I2C) |
| `embedded-io-async` | 0.7 | Async Read/Write traits (UART) |

### Peripherals with Async Support

| Module | Trait | Handler Functions | Notes |
|--------|-------|-------------------|-------|
| `timer.rs` | `embedded_hal_async::delay::DelayNs` | `on_pit{0-3}_interrupt()` | One AtomicWaker per PIT channel; ISR clears TIF, disables TIE |
| `gpio.rs` | `embedded_hal_async::digital::Wait` | `on_port{a-e}_interrupt()` | Per-port AtomicWaker + AtomicU32 pending flags; IRQC configured per-wait |
| `dma.rs` | `DmaChannel::wait_complete()` | `on_dma{0-15}_interrupt()` | Per-channel AtomicWaker; ISR clears CINT |
| `uart.rs` | `embedded_io_async::Read/Write` | `on_uart{0-2}_rx_tx_interrupt()` | Per-UART RX+TX wakers; ISR disables TIE/TCIE after waking TX |
| `spi.rs` | `embedded_hal_async::spi::SpiBus` | `on_spi{0,1}_interrupt()` | Per-instance AtomicWaker; uses TCF interrupt via RSER |
| `i2c.rs` | `embedded_hal_async::i2c::I2c` | `on_i2c{0,1}_interrupt()` | Per-instance AtomicWaker; ISR disables IICIE, driver reads status |

- [x] `Cargo.toml` — `async` feature flag with embassy-sync, embedded-hal-async, embedded-io-async
- [x] `timer.rs` — Async PIT delay with per-channel wakers
- [x] `gpio.rs` — Async GPIO Wait (rising/falling/any edge, high/low level) with per-port wakers + per-pin AtomicU32 pending flags
- [x] `dma.rs` — Async DMA wait_complete with per-channel wakers (4 on mk20d5, 16 on mk20d7)
- [x] `uart.rs` — Async serial Read/Write for Rx, Tx, and Serial types
- [x] `spi.rs` — Async SpiBus (read, write, transfer, transfer_in_place, flush)
- [x] `i2c.rs` — Async I2C transaction with full protocol (START, RSTART, ACK/NACK, STOP)
- [x] All 4 build combinations compile: mk20d7, mk20d5, mk20d7+async, mk20d5+async
- [x] Testsuite (blocking only) still compiles cleanly

---

## Phase 16: EEPROM / FlexMemory — COMPLETE

- [x] `Eeprom` struct with FlexRAM memory-mapped read/write API (`src/eeprom.rs`)
- [x] `FlashExt::flash()` returns `(Flash, Eeprom)` tuple (breaking change)
- [x] FlexRAM EEPROM mode detection (`is_eee_enabled()` via FCNFG.EEERDY)
- [x] Set FlexRAM function command (FTFL command 0x81)
- [x] EEERDY polling for write completion
- [x] `EepromError` enum (NotPartitioned, NotReady, OutOfBounds, AccessError, ProtectionViolation, CommandFailure)
- [x] Byte-level read/write (`read`, `write`) and bulk (`read_slice`, `write_slice`)
- [x] Unsafe partition command (`partition()` — one-time factory provisioning via FTFL command 0x80)
- [x] `capacity()` returns configured EEPROM size
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/eeprom.rs` (5 tests, conditional on partition state)

Reference: K20 ref manual chapter 29 (FTFL), chapter 30 (FlexMemory)

---

## Phase 17: Peripheral Improvements — COMPLETE

### 17.1 defmt Feature Flag — COMPLETE

- [x] `defmt = { version = "0.3", optional = true }` dependency
- [x] `#[cfg_attr(feature = "defmt", derive(defmt::Format))]` on all error/status types
- [x] Types covered: UART Error, SPI Error, I2C Error, DmaError, FlashError, CalibrationError, TimeInvalid, EepromError, all new enum types
- [x] Builds with and without `defmt` feature

### 17.2 Pin Validation Traits — COMPLETE

- [x] Sealed marker traits per peripheral function (Uart0TxPin, Spi0SckPin, I2c0SclPin, etc.)
- [x] Implementations for all valid pin-peripheral mappings from K20 reference manual ch10
- [x] UART, SPI, I2C constructors constrained to valid pin types
- [x] Compile-time rejection of invalid pin assignments

### 17.3 release() Methods — COMPLETE

- [x] `release()` on Serial<UART> (disables TE/RE, returns PAC type)
- [x] `release()` on Spi<SPI> (halts + disables module, returns PAC type)
- [x] `release()` on I2c<I2C> (disables IICEN, returns PAC type)
- [x] `release()` on Adc<ADC> (returns PAC type)
- [x] `release()` on Dac (disables DAC, returns pac::Dac0)
- [x] `release()` on Rtc (leaves running, returns pac::Rtc)
- [x] All release() methods are `unsafe` (caller must ensure no aliasing)

---

## Phase 18: DMA-Backed Peripheral Transfers — COMPLETE

- [x] `DmaTransfer<'a, CH>` lifetime-safe handle with abort-on-drop (`src/dma.rs`)
- [x] `is_complete()`, `has_error()`, `wait()` (blocks until done or error)
- [x] SPI + DMA: `write_dma()` (8-bit) and `write_dma_pushr()` (32-bit PUSHR) with DMAMUX routing (SPI0_TX/RX, SPI1_TX/RX)
- [x] UART + DMA: `write_dma()`, `read_dma()` with DMAMUX routing (UART0-2 TX/RX)
- [x] ADC + DMA: `read_dma()` continuous conversion with DMAMUX routing (ADC0, ADC1)
- [x] Per-instance DMA source constants passed via macro parameters
- [x] PDB driver (`src/pdb.rs`): `PdbExt` on `pac::Pdb0`, configurable prescaler/multiplier/modulus, continuous mode, pre-trigger enable/disable/delay, back-to-back mode, sequence error detection
- [x] ADC + PDB + DMA: `start_continuous_scan()` (1-2 channels) and `start_multi_channel_scan()` (3+ channels with DMA channel linking)

---

## Phase 19: FTM Input Capture / Output Compare — COMPLETE

- [x] `InputCapture<FTM, CH>` — configurable edge detection (rising/falling/both), capture(), wait(), interrupt control (`src/pwm.rs`)
- [x] `OutputCompare<FTM, CH>` — toggle/set/clear on match, set_compare(), interrupt control
- [x] `QuadratureDecoder<FTM>` — encoding modes (PhaseAB, count direction), count(), direction(), filter, polarity
- [x] `sealed::FtmInstance` trait for code sharing across FTM0/1/2
- [x] Enums: CaptureEdge, CompareAction, QuadMode, Direction (all with defmt support)
- [x] QuadratureDecoder restricted to FTM1/FTM2 (only instances with QDCTRL)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/pwm_advanced.rs` (9 tests, register-level OC/IC/Quad)

Reference: K20 ref manual chapter 36 (FTM)

---

## Phase 20: Low-Power Modes — COMPLETE

- [x] `PowerControl` driver wrapping SMC peripheral (`src/power.rs`)
- [x] `SmcExt` extension trait on `pac::Smc`
- [x] Wait mode (`enter_wait()` — WFI, wakes on any interrupt)
- [x] Stop modes (`enter_stop()` — NormalStop, VLPS, LLS, VLLS1/2/3)
- [x] VLPR mode entry/exit (`enter_vlpr()`, `exit_vlpr()`)
- [x] Mode protection (`allow_vlp()`, `allow_lls()`, `allow_vlls()`, `allow_all()`)
- [x] Stop abort detection (`stop_aborted()`)
- [x] Current mode query (`current_mode()`)
- [x] `Llwu` driver wrapping LLWU peripheral (`src/llwu.rs`)
- [x] `LlwuExt` extension trait on `pac::Llwu`
- [x] Pin wakeup source configuration (16 pins, rising/falling/any edge)
- [x] Module wakeup source configuration (LPTMR, CMP0, CMP1, RTC Alarm, RTC Seconds)
- [x] Wakeup flag reading and clearing (pin flags w1c, module flags read-only)
- [x] MCG BLPI mode transitions (`enter_blpi()`, `exit_blpi()`) in `src/clocks.rs`
- [x] `BlpiClocks` / `PeeState` types for type-safe mode management
- [x] `PeeState` saves/restores SIM CLKDIV1 register (works with any `ClockSpeed` preset)
- [x] SIM clock divider adjustment for VLPR limits (4 MHz core, 1 MHz bus/flash)
- [x] StopMode, PowerMode, WakeEdge, LlwuPin, LlwuModule enums (all with defmt support)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/power.rs` (3 tests, initial state + config)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/llwu.rs` (5 tests, register config)

Reference: K20 ref manual chapters 6 (PMC), 7 (LLWU), 8 (RCM), 15 (SMC)

---

## Phase 21: Additional Peripherals — COMPLETE

- [x] LPTMR (Low-Power Timer) driver (`src/lptmr.rs`)
  - [x] `LptmrExt` extension trait on `pac::Lptmr0`
  - [x] Time counter mode with configurable clock source (LPO 1kHz, ERCLK32K 32.768kHz, MCGIRCLK, OSCERCLK)
  - [x] Prescaler control (bypass or divide by 2..65536)
  - [x] `start()` with ms period, `start_raw()` for precise control
  - [x] `wait()` → nb::Result polling, `cancel()`, `count()`
  - [x] Interrupt enable/disable, flag clear
  - [x] CNR write-to-latch pattern for correct counter reads
- [x] CRC hardware accelerator driver (`src/crc_module.rs`)
  - [x] `CrcExt` extension trait on `pac::Crc`
  - [x] Configurable polynomial, seed, bit width (16/32), transpose modes
  - [x] Preset configurations: `CrcConfig::crc16_ccitt()`, `CrcConfig::crc32()`
  - [x] `configure()`, `feed()`, `result()`, `result_u16()`, `reset()`
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/crc.rs` (7 tests, known-vector CRC-16/CRC-32)
- [ ] Hardware validation: tests in `mk20dx-testsuite/tests/lptmr.rs` (8 tests, LPO 1kHz real-time)

---

## Decisions Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-02-19 | Single crate with feature flags (not separate crates per variant) | MK20D5 and MK20D7 share peripheral layouts; differ only in instance count and clock limits |
| 2026-02-19 | Target embedded-hal 1.0 only (no 0.2 compat) | 1.0 is stable; ecosystem migrating; avoids maintenance burden |
| 2026-02-19 | Const generics for GPIO (not macro-generated types) | Cleaner implementation, less code bloat, modern Rust idiom |
| 2026-02-19 | Blocking traits first, async later | Async requires DMA subsystem; validate blocking drivers on hardware first |
| 2026-02-19 | PAC as path dependency (not crates.io) | PAC not yet published; use `path = "../mk20dx-pac/mk20d7"` |
| 2026-02-19 | Extension traits for peripheral init (not free functions) | Surveyed 9 HAL crates (stm32f4xx, stm32f1xx, stm32h7xx, nrf, rp2040, esp, atsamd, lpc8xx, imxrt); extension traits (`constrain`/`freeze`/`split`) are the dominant ecosystem pattern. Benefits: discoverability via IDE autocomplete, ecosystem consistency, ownership enforcement, prelude-friendly. |
| 2026-02-21 | `defmt-test` v0.3 for hardware test harness (not `embedded-test`) | `embedded-test` v0.6/0.7 failed to compile on nightly 1.93.0 (missing modules, semihosting issues). `defmt-test` is stable, well-established, and integrates seamlessly with defmt logging. Trade-off: no per-test device reset (shared state), no `#[timeout]`/`#[should_panic]` attributes. |
| 2026-02-21 | Separate test binaries per peripheral area (not monolithic) | Each test binary can be run independently, isolating failures. Loopback tests (requiring wiring) are separate from self-tests. |
| 2026-02-22 | HAL exports `on_*_interrupt()` functions (not `#[interrupt]` handlers) | Avoids linker conflicts — user wires ISR to HAL handler. Standard pattern in embassy-stm32, embassy-nrf. |
| 2026-02-22 | `embassy-sync` AtomicWaker for async (not custom waker) | Battle-tested, executor-agnostic, minimal footprint. Works with Embassy, RTIC, or bare executors. |
| 2026-02-22 | Optional `async` feature flag (not always-on) | Keeps blocking-only builds dependency-free. Users opt in to embassy-sync + embedded-hal-async. |
