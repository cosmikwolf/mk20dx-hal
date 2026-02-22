# mk20dx-hal: Strategy

## Goal

Produce a correct, ergonomic Hardware Abstraction Layer crate for the MK20DX128 (Teensy 3.0) and MK20DX256 (Teensy 3.1/3.2) that implements `embedded-hal` 1.0 traits, built on top of the validated `mk20dx-pac` peripheral access crates.

The PAC is mature (Phase 5 complete, Phase 6 publishing in progress) with correctness patches validated against `kinetis.h` and semantic enum names across key peripherals (PORT MUX, FTM, ADC, MCG, SIM, DMA ATTR). PAC documentation lives at `../mk20dx-pac/docs/`.

---

## Phase 1: Project Scaffold

### 1.1 Crate Structure

Set up a standard `no_std` library crate with:

```toml
[package]
name = "mk20dx-hal"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
categories = ["embedded", "hardware-support", "no-std"]
keywords = ["arm", "cortex-m", "teensy", "kinetis", "nxp"]

[dependencies]
cortex-m = "0.7"
cortex-m-rt = { version = "0.7", optional = true }
embedded-hal = "1.0"
embedded-hal-nb = "1.0"
embedded-io = "0.6"
nb = "1.1"
fugit = "0.3"
critical-section = "1.1"

[dependencies.mk20d5]
path = "../mk20dx-pac/mk20d5"
optional = true

[dependencies.mk20d7]
path = "../mk20dx-pac/mk20d7"
optional = true

[features]
default = ["mk20d7"]
mk20d5 = ["dep:mk20d5", "mk20d5/rt"]
mk20d7 = ["dep:mk20d7", "mk20d7/rt"]
rt = []
```

### 1.2 Feature-Gated PAC Re-export

`lib.rs` re-exports whichever PAC is selected as `pac`:

```rust
#![no_std]

#[cfg(feature = "mk20d5")]
pub use mk20d5 as pac;

#[cfg(feature = "mk20d7")]
pub use mk20d7 as pac;

#[cfg(not(any(feature = "mk20d5", feature = "mk20d7")))]
compile_error!("Select a chip variant: mk20d5 or mk20d7");
```

### 1.3 Memory Layout

Provide `memory.x` linker scripts for each variant. Values from the K20 reference manuals and Teensy schematics:

**MK20DX128 (Teensy 3.0):**
```
MEMORY {
    FLASH : ORIGIN = 0x00000000, LENGTH = 128K
    RAM   : ORIGIN = 0x1FFFE000, LENGTH = 16K
}
```

**MK20DX256 (Teensy 3.1/3.2):**
```
MEMORY {
    FLASH : ORIGIN = 0x00000000, LENGTH = 256K
    RAM   : ORIGIN = 0x1FFF8000, LENGTH = 64K
}
```

A `build.rs` selects and copies the correct linker script based on feature flags.

### 1.4 Validation

- `cargo check --features mk20d7 --target thumbv7em-none-eabi` compiles
- `cargo check --features mk20d5 --target thumbv7em-none-eabi` compiles
- Both features simultaneously produces a compile error

---

## Phase 1.5: Flash Configuration & Watchdog

### 1.5.1 Flash Configuration Field

A 16-byte flash configuration field at 0x400-0x40F using `#[link_section = ".flashconfig"]`. Contains backdoor key, FPROT (flash protection disabled), FSEC (unsecured), FOPT (defaults). The `memory.x` linker script defines the `.flashconfig` section placement.

### 1.5.2 Watchdog Disable

The watchdog uses an extension trait on the PAC WDOG peripheral:

```rust
pub trait WdogExt {
    fn disable(self);
}

impl WdogExt for pac::Wdog {
    fn disable(self) {
        // Unlock: write 0xC520 then 0xD928 within 20 bus clocks
        // Disable: clear WDOGEN within 256 bus clocks of unlock
        cortex_m::interrupt::free(|_| { ... });
    }
}
```

This consumes the WDOG peripheral to prevent further access.

---

## Phase 2: Clock Configuration

**This is the critical-path foundation.** Almost every peripheral driver needs to know bus frequencies, and every peripheral needs its clock gate enabled before register access.

### 2.1 SIM Clock Gating

The System Integration Module (SIM) gates clocks to all peripherals via SCGC registers:
- `SCGC4`: UART0, UART1, UART2, I2C0, I2C1, CMP, USBOTG
- `SCGC5`: PORTA, PORTB, PORTC, PORTD, PORTE
- `SCGC6`: FTM0, FTM1, PIT, ADC0, RTC, DMAMUX, SPI0, FTM2 (mk20d7), ADC1 (mk20d7)
- `SCGC7`: DMA

Provide a safe API that enables peripheral clock gates. This can be as simple as a method on the SIM wrapper:
```rust
impl Sim {
    pub fn enable_clock<P: PeripheralClock>(&mut self) { ... }
}
```

Or it can be integrated into each peripheral's `::new()` method.

### 2.2 MCG Configuration

The Multipurpose Clock Generator produces the system clock from the external 16 MHz crystal on Teensy boards. The standard configuration path for Teensy:

1. Start in FEI (FLL Engaged Internal) mode after reset
2. Transition to FBE (FLL Bypassed External) — enable external crystal
3. Transition to PBE (PLL Bypassed External) — configure PLL
4. Transition to PEE (PLL Engaged External) — switch to PLL output

Target frequencies:
- MK20DX128 (Teensy 3.0): 48 MHz core, 48 MHz bus, 24 MHz flash
- MK20DX256 (Teensy 3.1/3.2): 72 MHz core, 36 MHz bus, 24 MHz flash

The PAC now provides semantic MCG enums for clock source selection (`c1().clks().fll()`, `c1().clks().internal()`, `c1().clks().external()`), FLL reference divider, oscillator range, and PLL VDIV — use these instead of raw bit patterns.

Reference: K20 ref manual chapter 24 (MCG), chapter 5 (Clock Distribution).

### 2.3 Extension Trait API

Clock configuration uses the extension trait pattern:

```rust
/// Extension trait on the PAC MCG peripheral.
pub trait McgExt {
    fn constrain(self) -> Mcg;
}

impl McgExt for pac::Mcg {
    fn constrain(self) -> Mcg { Mcg { _mcg: self } }
}

/// Wrapper that consumes the PAC MCG and provides `freeze()`.
pub struct Mcg { _mcg: pac::Mcg }

impl Mcg {
    /// Configure the clock tree and return frozen clock frequencies.
    /// Consumes the OSC peripheral; borrows SIM (shared with other modules).
    pub fn freeze(self, osc: OscPeripheral, sim: &pac::Sim) -> Clocks { ... }
}
```

### 2.4 Clocks Token

The `freeze()` method configures the MCG, sets SIM dividers, and returns a frozen `Clocks` struct:

```rust
pub struct Clocks {
    core_clk: Hertz,    // MCGOUTCLK / OUTDIV1
    bus_clk: Hertz,     // MCGOUTCLK / OUTDIV2
    flash_clk: Hertz,   // MCGOUTCLK / OUTDIV4
}

impl Clocks {
    pub fn core_clk(&self) -> Hertz { self.core_clk }
    pub fn bus_clk(&self) -> Hertz { self.bus_clk }
    pub fn flash_clk(&self) -> Hertz { self.flash_clk }
}
```

This struct is passed by reference to peripheral constructors for baud rate / prescaler calculation.

### 2.4 Validation

- Configure clocks and verify SysTick-based delays are reasonably accurate on hardware
- Verify peripheral clock gates work (accessing ungated peripheral should fault; gated should work)

---

## Phase 2.5: SysTick Delay

### 2.5.1 Delay Type

A `Delay` struct wrapping `cortex_m::peripheral::SYST` that implements `embedded_hal::delay::DelayNs`. Constructor requires `&Clocks` to compute cycles-per-microsecond.

```rust
pub struct Delay { syst: SYST, cycles_per_us: u32 }

impl Delay {
    pub fn new(syst: SYST, clocks: &Clocks) -> Self { ... }
    pub fn free(self) -> SYST { self.syst }
}
```

The SysTick reload register is 24 bits wide (max 0x00FFFFFF), so longer delays loop.

---

## Phase 3: GPIO

### 3.1 Architecture

Kinetis GPIO is split across two peripheral blocks that must be coordinated:

| Block | Registers | Purpose |
|-------|-----------|---------|
| **PORT** (PORTA..E) | PCR[0..31] (Pin Control Register) | Mux selection, pull, drive strength, interrupt config |
| **GPIO** (GPIOA..E) | PDOR, PSOR, PCOR, PTOR, PDIR, PDDR | Data output, set, clear, toggle, input, direction |

Each pin needs access to both its PORT PCR register and its GPIO port registers.

### 3.2 Type-State Design

Use const generics for port and pin number, with phantom types for mode:

```rust
pub struct Pin<const PORT: char, const N: u8, MODE = Input<Floating>> {
    _mode: PhantomData<MODE>,
}

// Mode types
pub struct Input<PULL>(PhantomData<PULL>);
pub struct Output<DRIVE>(PhantomData<DRIVE>);
pub struct Alternate<const MUX: u8>;

// Pull/drive types
pub struct Floating;
pub struct PullUp;
pub struct PullDown;
pub struct PushPull;
pub struct OpenDrain;
```

### 3.3 Port Splitting via Extension Trait

Each GPIO port is split into individual pin types via a `GpioExt` extension trait:

```rust
/// Extension trait on PAC PORT peripherals.
pub trait GpioExt {
    type Pins;
    /// Split the port into individual pin types.
    /// Consumes both PORT and GPIO peripherals; borrows SIM for clock gating.
    fn split(self, gpio: impl GpioPeriph, sim: &pac::Sim) -> Self::Pins;
}

impl GpioExt for pac::Porta {
    type Pins = PortAPins;
    fn split(self, gpio: pac::Pta, sim: &pac::Sim) -> PortAPins {
        sim.scgc5().modify(|_, w| w.porta()._1()); // Enable port clock
        PortAPins { pa0: Pin { _mode: PhantomData }, ... }
    }
}

pub struct PortAPins {
    pub pa0: Pin<'A', 0, Disabled>,
    pub pa1: Pin<'A', 1, Disabled>,
    // ...
    pub pa31: Pin<'A', 31, Disabled>,
}
```

This consumes the PAC's PORT and GPIO peripherals, preventing raw register access. The `GpioExt` trait is re-exported in the prelude for ergonomic use.

### 3.4 Pin Mux for Alternate Functions

When a peripheral (UART, SPI, etc.) claims a pin, it converts the pin to the correct alternate function:

```rust
// UART0 TX on PTA2 uses MUX=3 (ALT3)
let tx_pin = gpioa.pa2.into_alternate::<3>();
let serial = Serial::new(uart0, tx_pin, rx_pin, baud, &clocks);
```

The specific MUX values come from the K20 reference manual signal multiplexing table. These should be encoded as type-level constraints so invalid pin assignments are compile errors where practical.

The PAC now provides semantic PORT MUX enums (`Disabled`, `Gpio`, `Alt2`..`Alt7`) so the HAL can use `w.mux().gpio()` instead of `w.mux()._001()`.

### 3.5 embedded-hal Trait Implementations

| Trait | Implemented For | Notes |
|-------|----------------|-------|
| `InputPin` | `Pin<P, N, Input<PULL>>` | Reads PDIR bit |
| `OutputPin` | `Pin<P, N, Output<MODE>>` | Writes PSOR/PCOR (set/clear) |
| `StatefulOutputPin` | `Pin<P, N, Output<MODE>>` | Reads PDOR bit; `toggle()` via PTOR |

Error type: `Infallible` (GPIO operations cannot fail on this hardware).

### 3.6 Validation

- LED blink on Teensy pin 13 (PTB5 on Teensy 3.0, PTC5 on Teensy 3.1/3.2)
- Read a button input with pull-up
- Verify type-state prevents configuring an input pin as output without mode change

---

## Phase 4: UART / Serial

### 4.1 Architecture

MK20 has 3 UART peripherals. UART0 has a 8-byte FIFO; UART1 and UART2 have single-byte buffers.

All UART registers are 8-bit. The PAC correctly generates `u8`-sized register access.

### 4.2 Pin Assignment

UART pins are selected via PORT mux. Common Teensy mappings:

| UART | TX Pin | RX Pin | MUX | Teensy Pin |
|------|--------|--------|-----|------------|
| UART0 | PTA2 / PTB17 / PTD7 | PTA1 / PTB16 / PTD6 | 3 | 1/TX, 0/RX (default) |
| UART1 | PTC4 / PTE0 | PTC3 / PTE1 | 3 | 5, 21 |
| UART2 | PTD3 | PTD2 | 3 | 8, 7 |

### 4.3 Baud Rate Calculation

UART baud = (bus_clock or core_clock) / (SBR * (OSR + 1))

Where SBR is the 13-bit baud rate modulus (BDH[4:0] + BDL[7:0]) and OSR is the oversampling ratio in UART0_C4. UART0 is clocked from the core clock; UART1/2 from the bus clock.

### 4.4 Trait Implementations

| Crate | Trait | Notes |
|-------|-------|-------|
| `embedded-hal-nb` | `serial::Read<u8>` | Non-blocking read from RDR; returns `WouldBlock` if RDRF=0 |
| `embedded-hal-nb` | `serial::Write<u8>` | Non-blocking write to TDR; returns `WouldBlock` if TDRE=0 |
| `embedded-io` | `Read` | Blocking byte reads |
| `embedded-io` | `Write` | Blocking byte writes with `flush()` |

### 4.5 Validation

- Print to serial console via UART0 at 115200 baud
- Loopback test (TX→RX jumper)
- Verify baud rate accuracy across different system clock frequencies

---

## Phase 5: SPI

### 5.1 Architecture

MK20 uses DSPI (Deserial SPI) with hardware chip select and configurable frame sizes. SPI0 on both variants, SPI1 on mk20d7 only.

Key registers:
- **MCR**: Module configuration (master/slave, continuous clock, etc.)
- **CTAR0/CTAR1**: Clock and Transfer Attributes (baud rate, frame size, polarity, phase)
- **SR**: Status register (TCF, RFDF, TFFF flags)
- **PUSHR**: Push TX data with command bits (CS assertion, continuous transfer)
- **POPR**: Pop RX data

### 5.2 Baud Rate

DSPI baud = (bus_clock / PBR) * ((1 + DBR) / BR)

Where PBR (prescaler) and BR (baud rate scaler) are fields in CTARn, and DBR is the double baud rate bit.

### 5.3 Trait Implementation

Implement `embedded_hal::spi::SpiBus` for the HAL SPI type. Users compose `SpiDevice` via `embedded-hal-bus` wrappers (`ExclusiveDevice`, `RefCellDevice`, `CriticalSectionDevice`) which combine a `SpiBus` + CS `OutputPin`.

| Method | Implementation |
|--------|---------------|
| `read()` | Push dummy bytes to PUSHR, read from POPR |
| `write()` | Push bytes to PUSHR, discard POPR |
| `transfer()` | Push write bytes, collect read bytes |
| `transfer_in_place()` | In-place variant |
| `flush()` | Wait for TXCTR=0 and TCF |

### 5.4 Validation

- SPI loopback (MOSI→MISO jumper)
- Communication with a real SPI device (e.g., SPI flash, display)

---

## Phase 6: I2C

### 6.1 Architecture

MK20 I2C supports 7-bit and 10-bit addressing. I2C0 on both variants, I2C1 on mk20d7 only. All I2C registers are 8-bit.

Key registers:
- **F**: Frequency divider (ICR field selects from a lookup table of dividers)
- **C1**: Control (IICEN, MST, TX, TXAK)
- **S**: Status (BUSY, TCF, IICIF, ARBL, RXAK)
- **D**: Data register

### 6.2 Clock Rate

The I2C clock rate is derived from the bus clock using a divider table indexed by the ICR field in the F register. The K20 reference manual (chapter 37) contains the full divider table.

### 6.3 Trait Implementation

Implement `embedded_hal::i2c::I2c` with `SevenBitAddress`:

| Method | Implementation |
|--------|---------------|
| `read()` | START → address+R → read N bytes with ACK/NACK → STOP |
| `write()` | START → address+W → write bytes → STOP |
| `write_read()` | START → address+W → write → REPEATED START → address+R → read → STOP |
| `transaction()` | Compose read/write operations with repeated starts |

### 6.4 Validation

- I2C bus scan (detect devices at each address)
- Communication with a real I2C device (e.g., temperature sensor, OLED)

---

## Phase 7: Timers and Delay

### 7.1 SysTick Delay

The simplest `DelayNs` implementation uses the ARM Cortex-M SysTick timer (24-bit countdown). This is available via `cortex-m` crate and doesn't require any MK20-specific peripheral setup.

```rust
pub struct Delay {
    syst: cortex_m::peripheral::SYST,
    core_clk: Hertz,
}

impl embedded_hal::delay::DelayNs for Delay { ... }
```

### 7.2 PIT Timer

The Periodic Interrupt Timer has 4 independent 32-bit channels that can be chained for 64-bit operation. PIT is clocked from the bus clock.

Provide a HAL-specific timer API (no standard `embedded-hal` trait for countdown timers in 1.0):

```rust
pub struct PitChannel<const CH: u8> { ... }

impl PitChannel<CH> {
    pub fn start(&mut self, period: impl Into<Duration>) { ... }
    pub fn wait(&mut self) -> nb::Result<(), Infallible> { ... }
    pub fn cancel(&mut self) { ... }
}
```

### 7.3 Validation

- `DelayNs` produces reasonably accurate delays (verify with oscilloscope or logic analyzer)
- PIT periodic interrupt fires at expected rate

---

## Phase 8: PWM

### 8.1 Architecture

MK20 FlexTimer Modules (FTM) provide PWM output:
- **FTM0**: 8 channels (both variants)
- **FTM1**: 2 channels (both variants)
- **FTM2**: 2 channels (mk20d7 only)

FTM uses a 16-bit counter with configurable prescaler. PWM mode is selected per-channel via the CnSC register (MSB:MSA:ELSB:ELSA fields).

The PAC provides semantic FTM enums: clock source (`sc().clks().system()`, `.fixed()`, `.external()`) and prescaler (`sc().ps().div1()` through `.div128()`).

### 8.2 Trait Implementation

Implement `embedded_hal::pwm::SetDutyCycle` on individual FTM channels:

```rust
pub struct PwmChannel<FTM, const CH: u8> { ... }

impl SetDutyCycle for PwmChannel<FTM0, CH> {
    fn max_duty_cycle(&self) -> u16 { ... }      // MOD register value
    fn set_duty_cycle(&mut self, duty: u16) { ... } // CnV register
}
```

PWM frequency is set at the FTM level (shared MOD register); duty cycle is per-channel.

### 8.3 Validation

- PWM output on Teensy pin 13 LED (FTM channel) — should dim smoothly
- Frequency measurement with oscilloscope or logic analyzer

---

## Phase 9: ADC

### 9.1 Architecture

MK20 has a 16-bit SAR ADC. ADC0 on both variants, ADC1 on mk20d7 only. Supports hardware averaging, multiple resolutions (8/10/12/16-bit), and a calibration sequence.

No standard `embedded-hal` 1.0 trait for ADC. Provide a HAL-specific API. The PAC provides semantic ADC enums for resolution (`cfg1().mode().mode8_bit()` etc.), clock source, and sample time:

```rust
pub struct Adc<ADC> { ... }

impl Adc<ADC0> {
    pub fn new(adc: pac::ADC0, clocks: &Clocks) -> Self { ... }
    pub fn calibrate(&mut self) { ... }
    pub fn read(&mut self, channel: u8) -> u16 { ... }
    pub fn set_resolution(&mut self, res: Resolution) { ... }
}
```

### 9.2 Validation

- Read ADC value from a known voltage source
- Verify calibration sequence runs correctly
- Verify resolution switching works

---

## Phase 10: eDMA

### 10.1 Architecture

The eDMA (Enhanced Direct Memory Access) controller enables CPU-free data transfers between memory and peripherals. The DMAMUX routes peripheral request signals to DMA channels.

- MK20D5: 4 channels, 4 DMAMUX slots
- MK20D7: 16 channels, 16 DMAMUX slots

Both share identical register layouts, differing only in array sizes. The DMA module uses a `DmaChannel<const CH: u8>` zero-sized type per channel, following the same const generic pattern as PIT.

### 10.2 Transfer Model

Each DMA activation executes one *minor loop* (transfers NBYTES bytes). The *major loop* counts how many minor loop iterations to perform (CITER/BITER). After the major loop completes, DONE is set and optionally an interrupt fires. DREQ=1 auto-disables the hardware request on completion.

### 10.3 Extension Trait API

```rust
pub trait DmaExt: Sized {
    fn split(self, dmamux: pac::Dmamux, sim: &pac::Sim) -> DmaChannels;
}
```

The `split()` method consumes both DMA and DMAMUX peripherals, enables clock gates, configures default channel priorities (channel N = priority N, preemptable), and returns individual channel handles.

### 10.4 Scope

Implemented:
- Full TCD configuration (`configure()`)
- DMAMUX source routing (`set_source()`, `disable_source()`)
- Transfer control (`enable_request()`, `disable_request()`, `start()`)
- Status (`is_complete()`, `has_error()`, `is_active()`, `error_status()`)
- Flag/interrupt management
- Convenience methods (`configure_memcpy`, `configure_peripheral_read`, `configure_peripheral_write`)

Deferred to future phases:
- Scatter/gather (CSR.ESG, TCD chaining via DLASTSGA)
- Channel linking (CITER.ELINK, CSR.MAJORELINK)
- Minor loop offset mapping (CR.EMLM, NBYTES_MLOFF variants)
- Circular buffers (SMOD/DMOD)
- Safe Transfer type (borrow-based lifetime guarantee)
- Async integration (`embedded-hal-async`)

---

## Phase 11: USB Device

Implements `usb_device::bus::UsbBus` (v0.3) for the Kinetis USB-FS controller.

### 11.1 Hardware

- **USB0** at `0x4007_2000` — single instance, identical on both mk20d5 and mk20d7
- 16 endpoints, bidirectional, with ping-pong (EVEN/ODD) buffering
- BDT (Buffer Descriptor Table) in RAM, 512-byte aligned
- All USB registers are u8, `Safety = crate::Unsafe`
- ISTAT/ERRSTAT are w1c — must use `write()` not `modify()`

### 11.2 Clock Configuration

USB requires exactly 48 MHz. Derived from PLL via SIM:
- `SIM SOPT2`: `pllfllsel().pll()` + `usbsrc()._1()`
- `SIM CLKDIV2`: mk20d7 USBFRAC=1,USBDIV=2 (72×2/3=48); mk20d5 USBFRAC=0,USBDIV=0 (48×1=48)
- `SIM SCGC4`: `usbotg()._1()` clock gate

### 11.3 Implementation

- `UsbBus` struct with `UnsafeCell<Inner>` for interior mutability (trait requires `&self`)
- Static BDT (512-byte aligned) and buffer pool (64×64 bytes)
- `UsbBusExt` extension trait: `pac::Usb0.usb_bus(&sim)` → `UsbBus`
- Full `usb_device::bus::UsbBus` trait implementation
- Compatible with `usb-device` class crates (usbd-serial, usbd-hid, etc.)

### 11.4 Validation

```bash
cargo check --features mk20d7 --target thumbv7em-none-eabi
cargo check --no-default-features --features mk20d5 --target thumbv7em-none-eabi
```

Hardware: Flash to Teensy, verify USB enumeration.

---

## Phase 12: Hardware Validation Test Suite

### 12.1 Framework

Using `defmt-test` v0.3 with `probe-rs` runner for on-target test execution. Tests are organized as integration test binaries in `mk20dx-testsuite/tests/`, one per peripheral area.

Key properties:
- `#[defmt_test::tests]` macro on a module, `#[init]` for one-time setup, `#[test]` per test
- Shared `&mut State` across tests (init runs once per binary, no device reset between tests)
- `defmt-rtt` for structured logging, `panic-probe` for panic handling
- Standard `cargo test --test <binary>` workflow via probe-rs

### 12.2 Self-Tests (No External Wiring)

These tests validate peripheral drivers using only on-chip resources.

#### `tests/watchdog.rs` — 3 tests | Priority: CRITICAL

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_watchdog_disable` | Read STCTRLH after `disable()` | WDOGEN bit == 0 |
| `test_watchdog_survives_500ms` | Busy-wait 500ms after disable | Completes without reset |
| `test_system_functional_after_disable` | Read MCG.S register | Registers accessible |

#### `tests/clocks.rs` — 6 tests | Priority: CRITICAL

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_core_clk_72mhz` | `clocks.core_clk()` | == 72_000_000 Hz |
| `test_bus_clk_36mhz` | `clocks.bus_clk()` | == 36_000_000 Hz |
| `test_flash_clk_24mhz` | `clocks.flash_clk()` | == 24_000_000 Hz |
| `test_pll_locked` | Read MCG.S register | LOCK0=1, PLLST=1, CLKST=0b11 |
| `test_sim_dividers` | Read SIM.CLKDIV1 | OUTDIV1=0, OUTDIV2=1, OUTDIV4=2 |
| `test_osc_initialized` | Read MCG.S.OSCINIT0 | == 1 |

#### `tests/gpio.rs` — 8 tests | Priority: HIGH

Uses PTC5 (LED) and PTD4 (unconnected). First test uses typed pins; subsequent tests fall back to raw register access due to type-state pin consumption in shared-state model.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_output_set_high` | PTC5 push-pull output, set high | `is_set_high() == true` |
| `test_output_set_low` | PTC5 set low (raw regs) | PDOR bit == 0 |
| `test_output_toggle` | PTC5 low → toggle (raw regs) | PDOR bit == 1 |
| `test_pull_up_reads_high` | PTD4 pull-up input | PDIR bit == 1 |
| `test_pull_down_reads_low` | PTD4 pull-down input (raw regs) | PDIR bit == 0 |
| `test_mode_transition_out_to_in` | PTC5 output → input (raw regs) | No fault |
| `test_mode_transition_in_to_out` | PTD4 input → output (raw regs) | No fault |
| `test_open_drain_set_low` | PTC5 open-drain (raw regs) | PDOR bit == 0 |

#### `tests/delay.rs` — 7 tests | Priority: HIGH

Uses PIT as independent timing reference to cross-validate SysTick delays.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_delay_1ms_completes` | `delay.delay_ms(1)` | Completes |
| `test_delay_100ms_completes` | `delay.delay_ms(100)` | Completes |
| `test_delay_1s_completes` | `delay.delay_ms(1000)` — exercises SysTick loop path | Completes |
| `test_delay_ns_1us` | `delay.delay_ns(1000)` | Completes |
| `test_delay_zero` | `delay.delay_ns(0)` | Completes immediately |
| `test_delay_10ms_pit_crosscheck` | PIT free-run → delay 10ms → read PIT delta | Within ±5% of 360k ticks |
| `test_delay_100ms_pit_crosscheck` | PIT free-run → delay 100ms → read PIT delta | Within ±2% of 3.6M ticks |

#### `tests/timer.rs` — 10 tests | Priority: HIGH

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_start_and_wait` | ch0 1ms period, `nb::block!(wait())` | Returns `Ok(())` |
| `test_wait_would_block` | ch0 1s period, immediate poll | `Err(WouldBlock)` |
| `test_cancel` | ch0 1s, cancel, poll | `Err(WouldBlock)` |
| `test_current_counts_down` | ch0 1s, read current twice with delay | second < first |
| `test_has_expired` | ch0 1ms, check before + delay 5ms + check after | Before=false, After=true |
| `test_clear_interrupt_flag` | After expiry, clear, check | `has_expired() == false` |
| `test_enable_disable_interrupt` | Enable TIE, verify set; disable, verify cleared | Register bits match |
| `test_channels_independent` | ch0=10ms, ch1=50ms, wait ch0, check ch1 not expired | Correct ordering |
| `test_reload_after_expiry` | Expire, restart, expire again | Both waits succeed |
| `test_start_ticks_raw` | `start_ticks(36_000)` = 1ms at 36MHz | Completes |

#### `tests/adc.rs` — 10 tests | Priority: HIGH

Internal ADC channels provide known reference voltages — the strongest self-tests in the suite.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_calibration_succeeds` | `adc.calibrate()` in init | Reaches test |
| `test_vrefsl_reads_zero` | Channel 30 (VSSA/GND) at 10-bit | Value < 50 |
| `test_vrefsh_reads_max` | Channel 29 (VDDA/3.3V) at 10-bit | Value > 974 |
| `test_bandgap_in_range` | Channel 27 (~1.0V) at 10-bit | 250–370 |
| `test_temperature_in_range` | Channel 26 (temp sensor) | 100–900 (loose bounds) |
| `test_resolution_8bit` | 8-bit mode, VREFSH | 245–255 |
| `test_resolution_12bit` | 12-bit, VREFSH | > 3995 |
| `test_resolution_16bit` | 16-bit, VREFSH | > 64500 |
| `test_averaging_reduces_noise` | 10× raw vs 10× avg32 on bandgap, compare variance | avg_var ≤ raw_var |
| `test_sequential_channels` | Read VREFSL, bandgap, VREFSH in sequence | All within ranges |

#### `tests/dma.rs` — 12 tests | Priority: MEDIUM

All memory-to-memory — no external peripherals needed.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_memcpy_4_bytes` | Copy [1,2,3,4] via DMA | dst == [1,2,3,4] |
| `test_memcpy_256_bytes` | Copy 256-byte pattern | All match |
| `test_memcpy_1024_bytes` | Larger transfer | All match |
| `test_aligned_uses_32bit` | 4-byte aligned, check TCD ATTR | SSIZE/DSIZE = Bits32 |
| `test_unaligned_uses_8bit` | Odd-aligned address, check TCD ATTR | SSIZE/DSIZE = Bits8 |
| `test_not_complete_before_start` | Configure only, no start | `is_complete() == false` |
| `test_complete_after_transfer` | Configure + start + wait | `is_complete() == true` |
| `test_clear_done_flag` | After complete, `clear_done()` | `is_complete() == false` |
| `test_multiple_channels` | ch0 + ch1 independent memcpy | Both dst correct |
| `test_source_routing` | Set DMAMUX to ALWAYS_ON0, verify CHCFG | Source=54, ENBL=1 |
| `test_disable_source` | Set then disable source | CHCFG.ENBL == 0 |

#### `tests/pwm.rs` — 7 tests | Priority: MEDIUM

Register validation only — no oscilloscope needed.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_max_duty_nonzero` | FTM1 at 1kHz, `max_duty_cycle()` | > 0 |
| `test_max_duty_reasonable` | 1kHz @ 36MHz bus → MOD ~35999 | 35000–36500 |
| `test_set_duty_zero` | `set_duty_cycle(0)` | Returns `Ok(())` |
| `test_set_duty_max` | `set_duty_cycle(max)` | Returns `Ok(())` |
| `test_set_duty_half` | `set_duty_cycle(max/2)`, read CnV | CnV ≈ max/2 |
| `test_enable_channel` | Enable, verify CnSC MSB+ELSB bits | Bits set |
| `test_ftm_sc_register` | Verify SC CLKS=System + PS=div1 | Register match |

#### `tests/i2c.rs` — 4 tests | Priority: MEDIUM

**Requires 4.7kΩ pull-up resistors on PTB0 (SCL) and PTB1 (SDA) to 3.3V.**

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_write_nack_on_empty_bus` | Write to addr 0x50 | `Err(AddressNack)` |
| `test_read_nack_on_empty_bus` | Read from addr 0x50 | `Err(AddressNack)` |
| `test_bus_recovers_after_nack` | NACK, then retry | Second also NACKs (not hang) |
| `test_scan_empty_bus` | Probe addrs 0x08–0x77 | All 112 return NACK |

#### `tests/usb.rs` — 6 tests | Priority: LOW

Init/allocation tests only — no USB host needed.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_clock_gate_enabled` | Check SIM.SCGC4.USBOTG | == 1 |
| `test_clock_48mhz_source` | Check SIM.SOPT2 PLL + USBSRC | Bits correct |
| `test_clock_divider` | Check SIM.CLKDIV2 | USBFRAC=1, USBDIV=2 |
| `test_alloc_ep0_control` | Allocate EP0 control | Succeeds |
| `test_alloc_bulk_endpoints` | Allocate bulk IN + OUT | Both succeed |
| `test_enable_no_crash` | Call `enable()` | No fault |

#### `tests/flash.rs` — 6 tests | Priority: HIGH (read-only)

**Read-only and error-path tests only.** Erase/write tests are deliberately omitted — erasing the wrong sector bricks the chip and requires mass erase recovery via J-Link. A round-trip erase-write-read test targeting the last flash sector may be added later with explicit opt-in (e.g., `--features destructive-flash-test`).

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_capacity` | `flash.capacity()` | == 262144 (mk20d7) |
| `test_security_unsecured` | `security_status() & 0x3` | == 0b10 (unsecured) |
| `test_safety_floor_rejects_erase` | `erase_sector(0x000)` | `Err(Protected)` |
| `test_safety_floor_rejects_write` | `program_longword(0x400, ..)` | `Err(Protected)` |
| `test_not_aligned_rejected` | `program_longword(0x801, ..)` | `Err(NotAligned)` |
| `test_read_vector_table` | `ReadNorFlash::read()` at offset 0, 16 bytes | First 4 bytes (initial SP) are non-zero |

#### `tests/dac.rs` — 5 tests | Priority: MEDIUM

Register-level validation only. Verifying actual analog output voltage requires an oscilloscope or ADC loopback (DAC0_OUT → ADC pin).

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_clock_gate_enabled` | Check SIM.SCGC2.DAC0 after init | == 1 |
| `test_init_value_zero` | `get_value()` immediately after `dac()` | == 0 |
| `test_set_get_roundtrip` | `set_value(2048)` then `get_value()` | == 2048 |
| `test_12bit_mask` | `set_value(0xFFFF)` then `get_value()` | == 0x0FFF (masked) |
| `test_enable_disable` | `disable()`, read C0.DACEN; `enable()`, read C0.DACEN | 0 then 1 |

#### `tests/rtc.rs` — 7 tests | Priority: MEDIUM

Uses the on-board 32.768 kHz crystal. The oscillator may take up to 500 ms to start after first power-on, but is typically already running if VBAT is maintained. Tests that depend on counting use a SysTick delay to wait.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_clock_gate_enabled` | Check SIM.SCGC6.RTC after init | == 1 |
| `test_oscillator_enabled` | Read RTC.CR.OSCE after init | == 1 |
| `test_set_time_and_read` | `set_time(1000)`, then `seconds()` | Ok(1000) or Ok(1001) |
| `test_counter_increments` | Read seconds, delay ~2s, read again | second > first |
| `test_time_valid_after_set` | `set_time(500)`, `time_is_valid()` | == true |
| `test_alarm_flag` | `set_time(100)`, `set_alarm(101)`, delay ~2s | `alarm_fired() == true` |
| `test_disable_stops_counter` | `disable()`, read, delay ~2s, read | Values equal |

#### `tests/cmp.rs` — 7 tests | Priority: MEDIUM

Uses the CMP internal 6-bit DAC as a self-referencing test source. By connecting both inputs to the internal DAC (IN7) or setting known DAC levels, we can validate comparator behavior without external wiring.

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_clock_gate_enabled` | Check SIM.SCGC4.CMP after init | == 1 |
| `test_enabled_after_init` | Read CMP0 CR1.EN | == 1 |
| `test_internal_dac_high_vs_low` | Plus=INTERNAL_DAC @ level 63, minus=INTERNAL_DAC disabled (IN0, floating) — compare with known asymmetry | Output is deterministic (does not crash) |
| `test_hysteresis_register` | Set each hysteresis level (0-3), read CR0.HYSTCTR | Matches for all 4 levels |
| `test_invert_flips_output` | Read `output()`, `set_inverted(true)`, read again | Different (or both stable) |
| `test_disable_enable` | `disable()` → read CR1.EN=0, `enable()` → read CR1.EN=1 | Both match |
| `test_clear_flags` | `clear_flags()`, read SCR | CFR=0 and CFF=0 |

### 12.3 Loopback Tests (Require Wiring)

#### Wiring Requirements

| Wire | From (Teensy Pin) | To (Teensy Pin) | Test Binary |
|------|-------------------|-----------------|-------------|
| GPIO loopback | PTD5 (pin 20) | PTD6 (pin 21) | `gpio_loopback.rs` |
| UART2 loopback | PTD3 (pin 8, TX) | PTD2 (pin 7, RX) | `uart_loopback.rs` |
| SPI0 loopback | PTC6 (pin 11, MOSI) | PTC7 (pin 12, MISO) | `spi_loopback.rs` |

#### `tests/gpio_loopback.rs` — 2 tests

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_output_drives_input` | PTD5 output → PTD6 input. High/low | PTD6 reads match |
| `test_toggle_reflected` | Toggle PTD5 10×, verify PTD6 follows | All 10 match |

#### `tests/uart_loopback.rs` — 5 tests

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_single_byte` | Write 0xA5 TX → read RX | Read == 0xA5 |
| `test_multiple_bytes` | Write [0x01..0x04] → read | All match |
| `test_all_byte_values` | Write 0x00..0xFF → read | All 256 match |
| `test_split_tx_rx` | Raw register loopback | Byte matches |
| `test_rx_empty_returns_wouldblock` | Read with nothing sent | `Err(WouldBlock)` |

#### `tests/spi_loopback.rs` — 7 tests

| Test | Description | Pass Criteria |
|------|-------------|---------------|
| `test_transfer_in_place` | 1 byte 0xA5 via `transfer_in_place` | Read == 0xA5 |
| `test_transfer_multiple` | 4 bytes in-place | All match |
| `test_transfer_separate_bufs` | `transfer(read, write)` | read == write |
| `test_all_byte_values` | Transfer 0x00..0xFF | All match |
| `test_read_sends_zeros` | `read()` into buffer | All == 0x00 |
| `test_write_completes` | `write()` 4 bytes | No hang |
| `test_flush` | `flush()` after write | Completes |

### 12.4 Tests Requiring Additional Hardware (Not Implemented)

These tests cannot be performed with the current test suite and would require additional equipment.

| Category | What | Why | Hardware Needed |
|----------|------|-----|----------------|
| I2C device communication | Read/write to real I2C slave | Bus protocol needs a responding device | I2C EEPROM (AT24C32) + 4.7kΩ pull-ups |
| USB enumeration | Verify device appears on host | Requires USB host handshake | USB cable + host PC running `lsusb` |
| USB data transfer | End-to-end host↔device I/O | Full CDC/HID class testing | USB cable + host test script |
| PWM frequency accuracy | Measure actual output waveform | Register validation can't confirm analog output | Oscilloscope or logic analyzer |
| PWM duty cycle accuracy | Measure analog duty cycle | Need time-domain measurement | Oscilloscope |
| Delay absolute accuracy | Verify actual wall-clock timing | PIT crosscheck validates relative but not absolute | Oscilloscope or frequency counter |
| ADC external pin accuracy | Measure real-world voltages | Internal refs only test internal channels | Known voltage source + wiring to ADC pin |
| UART baud rate accuracy | Verify actual bit timing | Loopback proves data integrity, not timing | Logic analyzer |
| DMA peripheral transfers | DMA with SPI/UART/ADC triggers | Memory-to-memory only without peripheral source | Configured peripheral + peripheral-specific wiring |
| SPI with real device | Protocol correctness with slave | Loopback tests framing, not CS/protocol | SPI flash or EEPROM |
| Interrupt-driven operation | Verify ISR execution | Tests are polling-only | Embassy async integration + interrupt tests |
| Multi-core/multi-task | Concurrent peripheral access | Single-threaded tests only | RTOS or async executor |

### 12.5 Running Tests

```bash
cd mk20dx-testsuite

# Self-tests (no wiring needed)
cargo test --test watchdog
cargo test --test clocks
cargo test --test gpio
cargo test --test delay
cargo test --test timer
cargo test --test adc
cargo test --test dma
cargo test --test pwm

# Requires pull-ups on PTB0/PTB1
cargo test --test i2c

# Requires loopback wires
cargo test --test gpio_loopback   # PTD5 → PTD6
cargo test --test uart_loopback   # PTD3 → PTD2
cargo test --test spi_loopback    # PTC6 → PTC7

# Run all
cargo test
```

---

## Phase 13: Flash Memory (FTFL)

### 13.1 Hardware

The FTFL controller provides erase and program access to the internal program flash:

- **FTFL** at `0x4002_0000` — identical register layout on both mk20d5 and mk20d7
- **Program flash**: 128 KB (mk20d5) / 256 KB (mk20d7), memory-mapped at `0x0000_0000`
- **Sector size**: 2 KB (smallest erase unit)
- **Write unit**: 4 bytes (longword), must be longword-aligned
- Single flash block — cannot read program flash while a command is running

### 13.2 Critical Constraints

1. **RAM execution**: The function that launches commands and polls CCIF must execute from RAM (`#[link_section = ".data"]`) because flash is unreadable while CCIF=0.
2. **Flash config protection**: Sector 0 contains the flash config field (0x400-0x40F). Erasing it without restoring FSEC=0xFE bricks the chip. A safety floor at 0x800 prevents all writes below sector 1.
3. **Critical section**: ISR code lives in flash, so flash commands must run inside `cortex_m::interrupt::free()`.
4. **No cumulative programming**: A longword can only be written once after erase.

### 13.3 Extension Trait API

```rust
pub trait FlashExt {
    fn flash(self) -> Flash;
}
impl FlashExt for pac::Ftfl { ... }
```

The `Flash` struct is zero-sized (all state in hardware registers). Consuming the PAC `Ftfl` prevents aliased access. Provides HAL-specific methods (`erase_sector`, `program_longword`, `is_protected`, `security_status`) plus `embedded_storage::nor_flash` trait implementations (`ReadNorFlash`, `NorFlash`).

### 13.4 Validation

On-target testing requires extreme care. Initial validation should use a sector well above the firmware (e.g., last sector of flash) and verify round-trip erase-write-read.

---

## Phase 14: DAC, RTC, CMP

### 14.1 DAC (mk20d7 only)

Single-instance driver for the 12-bit DAC0 peripheral. Entire module gated with `#[cfg(feature = "mk20d7")]`. Simple single-instance pattern (no macro needed), similar to watchdog.

API: `DacExt::dac(sim)` → `Dac` with `set_value(u16)`, `get_value()`, `set_vref()`, `enable()`, `disable()`.

Init defaults: DACEN=1, VREF1 (VDDA), software trigger, buffer disabled, high power mode, output=0. No `Clocks` parameter.

### 14.2 RTC (both variants)

Single-instance driver for the Real-Time Clock. Uses independent 32.768 kHz oscillator (on-board on Teensy 3.x). Key constraint: TSR can only be written when TCE=0.

API: `RtcExt::rtc(sim)` → `Rtc` with `seconds()` → `Result<u32, TimeInvalid>`, `set_time(u32)`, alarm support, interrupt control.

Init preserves existing time if VBAT maintained. Enables oscillator with ~10 pF caps, disables interrupts, starts counter if valid.

### 14.3 CMP (multi-instance)

Macro-generated driver following the `adc.rs` pattern. CMP0+CMP1 on both variants, CMP2 on mk20d7 only. All share a single SIM clock gate bit (SCGC4.CMP).

Critical w1c hazard in SCR register: CFF (bit 1) and CFR (bit 2) are w1c flags mixed with r/w config bits. All SCR writes use `write()` with manual config bit preservation to avoid accidentally clearing flags.

API: `CmpExt::cmp(plus, minus, sim)` → `Cmp<Instance>` with output reading, input mux, internal 6-bit DAC, hysteresis, edge detection, and interrupt control.

---

## Phase 15+: Extended Peripherals (Future)

| Peripheral | Crate/Trait | Notes |
|-----------|-------------|-------|
| Async | `embedded-hal-async` | Async trait variants for all peripherals |

---

## Design Decisions

### Extension Traits for Peripheral Initialization

Following the dominant pattern across the embedded Rust ecosystem (stm32f4xx-hal, stm32f1xx-hal, stm32h7xx-hal, nrf-hal, rp2040-hal, lpc8xx-hal, imxrt-hal), peripherals are initialized via **extension traits** on PAC types:

- **`McgExt`** on `pac::Mcg` — provides `constrain()` returning an MCG builder, which has a `freeze()` method that configures clocks and returns the `Clocks` token
- **`GpioExt`** on port types — provides `split()` that consumes both PORT and GPIO peripherals, enables the SIM clock gate, and returns individual pin structs
- **`WdogExt`** on `pac::Wdog` — provides `disable()` that consumes the watchdog peripheral

This pattern was chosen after surveying 9 HAL crates. Benefits:
1. **Discoverability** — users type `peripheral.` and see available methods via IDE autocomplete
2. **Ecosystem consistency** — developers familiar with any other HAL instantly know the API shape
3. **Ownership enforcement** — `constrain()`/`split()` consume the PAC peripheral, preventing raw register access
4. **Prelude-friendly** — extension traits are re-exported in the prelude so `use mk20dx_hal::prelude::*` makes them available

Example usage:
```rust
use mk20dx_hal::prelude::*;
let dp = pac::Peripherals::take().unwrap();
dp.WDOG.disable();
let clocks = dp.MCG.constrain().freeze(dp.OSC, &dp.SIM);
let pins_a = dp.PORTA.split(dp.PTA, &dp.SIM);
```

### Variant Selection via Feature Flags (not separate crates)

Both MK20DX128 and MK20DX256 share the same peripheral register layout — they differ only in peripheral instance counts and clock limits. A single HAL crate with feature flags avoids code duplication. Feature-gated `cfg` blocks expose additional peripherals on the larger variant.

### Const Generics for GPIO (not macros)

Modern Rust (1.51+) supports const generics. Using `Pin<const PORT: char, const N: u8, MODE>` instead of macro-generated `PA0`, `PA1`, etc. reduces code bloat and makes the implementation more readable. Trade-off: slightly more verbose type annotations in user code.

### embedded-hal 1.0 Only (no 0.2 compatibility)

Target `embedded-hal` 1.0 exclusively. The 1.0 API is stable and the ecosystem is migrating. No compatibility shim for 0.2 traits — keeps the implementation simple.

### Blocking First (async later)

Implement blocking `embedded-hal` traits first. Async (`embedded-hal-async`) can be added in a future phase now that the blocking drivers and DMA subsystem (Phase 10) are complete.

### Error Types

- GPIO: `Infallible` (register writes cannot fail)
- UART/SPI/I2C: Peripheral-specific error enums implementing the `embedded-hal::*::Error` traits with appropriate `ErrorKind` mappings
- Delay: `Infallible`

---

## Reference Implementations

Study these HALs for architectural patterns:

| HAL | Relevance | Notes |
|-----|-----------|-------|
| [stm32f4xx-hal](https://github.com/stm32-rs/stm32f4xx-hal) | Gold standard | Cortex-M4, comprehensive type-state GPIO, excellent clock config |
| [imxrt-hal](https://github.com/imxrt-rs/imxrt-hal) | Most mature NXP HAL | Multi-chip family, good architecture for feature-gated variants |
| [mkw41z-hal](https://github.com/therealprof/mkw41z-hal) | Same Kinetis family | Similar PORT/GPIO split, MCG clock tree |
| [kea-hal](https://github.com/wcpannell/kea-hal) | Kinetis KEA | Simpler Kinetis part, same patterns |

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Clock tree configuration is wrong | Medium | High | Cross-reference against Teensy 3.x Arduino core startup code (`mk20dx128.c`) which is known-working |
| GPIO pin mux table has errors | Low | Medium | Verify against K20 ref manual signal multiplexing table |
| Baud rate calculations are off | Medium | Medium | Test at standard baud rates (9600, 115200); compare against Arduino core calculations |
| I2C divider table is incomplete | Low | Medium | Full table is in K20 ref manual chapter 37; verify against Arduino Wire library |
| Feature-gated compilation breaks one variant | Medium | Low | CI checks both variants |
| PAC has undiscovered register bugs | Low | Medium | PAC is validated against kinetis.h with 0 address mismatches; all known Kinetis SVD bugs verified absent; hardware testing will catch remaining issues |

---

## Open Questions

1. **Memory layout for Teensy bootloader:** Teensy uses a custom HalfKay bootloader. Does the flash start address need to account for a bootloader offset, or is the bootloader in a separate flash region?

2. **Flash configuration field:** The MK20 has a 16-byte flash configuration field at 0x400-0x40F. The Teensy Arduino core writes specific values here. Do we need to provide this in the HAL or is it the application's responsibility?

3. **Pin mapping table:** Should the HAL include a Teensy-specific pin mapping (Arduino pin numbers → port/pin pairs), or should that be a separate board support crate?

4. **USB bootloader interaction:** Teensy boards enter the bootloader via USB. Does the HAL need to support this, or is it handled at a lower level?
