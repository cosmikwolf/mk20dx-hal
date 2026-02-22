# mk20dx-hal

Hardware Abstraction Layer for the NXP Kinetis MK20DX128 (Teensy 3.0) and MK20DX256 (Teensy 3.1/3.2), implementing `embedded-hal` 1.0 traits on top of the [mk20dx-pac](../mk20dx-pac/) peripheral access crates.

## Project Context

This HAL consumes the `mk20d5` and `mk20d7` PAC crates (register-level access) and provides safe, ergonomic APIs for each peripheral. It implements standard `embedded-hal` traits so generic drivers work out of the box.

```
Application / Generic Drivers (use embedded-hal traits)
        |
   mk20dx-hal           <-- This crate: safe APIs + trait impls
        |
   mk20d5 / mk20d7      <-- PAC crates (../mk20dx-pac/)
        |
   Hardware Registers
```

See `STRATEGY.md` for the full implementation plan and `STATUS.md` for current progress.

## Target Hardware

| Feature Flag | PAC Crate | Chip | Board | Flash | RAM | Clock | DMA Ch |
|-------------|-----------|------|-------|-------|-----|-------|--------|
| `mk20d5` | `mk20d5` | MK20DX128VLH5 | Teensy 3.0 | 128K | 16K | 48 MHz | 4 |
| `mk20d7` | `mk20d7` | MK20DX256VLH7 | Teensy 3.1/3.2 | 256K | 64K | 72 MHz | 16 |

Both are ARM Cortex-M4 (no FPU), target `thumbv7em-none-eabi`.

## Sibling Project: mk20dx-pac

The PAC lives at `../mk20dx-pac/`. It is mature (Phase 5 complete, Phase 6 publishing in progress), validated against `kinetis.h`, and both variants compile cleanly. Ergonomics patches provide semantic enum names across PORT MUX, FTM, ADC, MCG, SIM, and DMA ATTR fields — the HAL can use these directly instead of raw bit patterns.

Key PAC documentation:
- `../mk20dx-pac/CLAUDE.md` — PAC project guide, build commands, SVD patching conventions
- `../mk20dx-pac/docs/STATUS.md` — PAC validation results, patch inventory, known bugs verified absent
- `../mk20dx-pac/docs/STRATEGY.md` — PAC implementation strategy and phase breakdown
- `../mk20dx-pac/docs/RESEARCH_FINDINGS.md` — Hardware details, SVD quality analysis, reference sources
- `../mk20dx-pac/README.md` — PAC usage examples and patch summary
- `../mk20dx-pac/reference/kinetis.h` — Ground truth for register definitions

## Reference Manuals

The K20 reference manuals have been extracted to per-chapter markdown in the PAC crate. These are gitignored but regeneratable (see PAC's `docs/STATUS.md` for the command).

| Sub-Family | Extracted Chapters | Source PDF |
|-----------|-------------------|------------|
| 72 MHz (MK20D7) | `../mk20dx-pac/reference/refman_chapters/` (51 files) | K20P64M72SF1RM |
| 50 MHz (MK20D5) | `../mk20dx-pac/reference/refman_50mhz_chapters/` (49 files) | K20P64M50SF0RM |

Key chapters for HAL development:
- Chapter 5: Clock Distribution (MCG output → SIM dividers → peripheral clocks)
- Chapter 11: Port Control and Interrupts (PORT mux, pull, drive strength)
- Chapter 12: System Integration Module (SIM clock gating, SCGC registers)
- Chapter 24: MCG (Multipurpose Clock Generator — FLL/PLL configuration)
- Chapter 35-37: UART, SPI (DSPI), I2C
- Chapter 36: FTM (FlexTimer — PWM, input capture)
- Chapter 31: ADC (16-bit SAR)
- Chapter 21: eDMA (Direct Memory Access)
- Chapter 28: PIT (Periodic Interrupt Timer)
- Chapter 34: USB OTG / USB-FS (Full-Speed USB Device)

## Directory Structure

```
mk20dx-hal/
├── Cargo.toml
├── build.rs                # Copies memory.x for the selected variant
├── memory/
│   ├── memory_mk20d5.x    # Linker script for Teensy 3.0
│   └── memory_mk20d7.x    # Linker script for Teensy 3.1/3.2
├── src/
│   ├── lib.rs              # Feature gates, PAC re-export, module declarations
│   ├── prelude.rs          # Glob import of commonly-used traits + extension traits
│   ├── adc.rs              # AdcExt, ADC driver (HAL-specific, no standard trait)
│   ├── clocks.rs           # McgExt, MCG+SIM clock configuration, Clocks token
│   ├── delay.rs            # DelayNs via SysTick
│   ├── dma.rs              # DmaExt, eDMA+DMAMUX driver (HAL-specific)
│   ├── flash_config.rs     # 16-byte flash configuration field at 0x400
│   ├── gpio.rs             # GpioExt, pin type-states, PORT mux, embedded-hal digital
│   ├── i2c.rs              # I2cExt, I2C driver, embedded-hal I2c
│   ├── pwm.rs              # FtmExt, FTM-based PWM, embedded-hal SetDutyCycle
│   ├── spi.rs              # SpiExt, DSPI driver, embedded-hal SpiBus
│   ├── time.rs             # Re-exports fugit types for frequencies/durations
│   ├── timer.rs            # PitExt, PIT timer abstractions
│   ├── uart.rs             # UartExt, UART driver, embedded-hal-nb serial, embedded-io
│   ├── usb.rs              # UsbBusExt, USB device driver, usb-device UsbBus
│   └── watchdog.rs         # WdogExt, watchdog disable
├── CLAUDE.md
├── STRATEGY.md
└── STATUS.md
```

## Chip Variant Selection

Exactly one feature flag must be enabled:
- `mk20d5` — MK20DX128 (Teensy 3.0): 4 DMA ch, 1 SPI, 1 I2C, 2 FTM, 1 ADC, 1 USB
- `mk20d7` — MK20DX256 (Teensy 3.1/3.2): 16 DMA ch, 2 SPI, 2 I2C, 3 FTM, 2 ADC, 1 USB

Feature gates control which PAC crate is pulled in and which peripheral instances are exposed. Both variants share the same driver implementations — only peripheral counts and clock limits differ.

## Key Design Patterns

### Extension Traits for Peripheral Initialization

Following the dominant embedded Rust ecosystem pattern (stm32f4xx-hal, nrf-hal, rp2040-hal, etc.), peripherals are initialized via extension traits on PAC types. These traits are re-exported in the prelude.

| Extension Trait | PAC Type | Method | Returns |
|----------------|----------|--------|---------|
| `McgExt` | `pac::Mcg` | `constrain()` → `Mcg` → `freeze(osc, &sim)` | `Clocks` |
| `GpioExt` | `pac::Porta`, etc. | `split(gpio, &sim)` | `PortAPins`, etc. |
| `WdogExt` | `pac::Wdog` | `disable()` | `()` |
| `UartExt` | `pac::Uart0`, etc. | `serial(tx, rx, baud, clocks, sim)` | `Serial<Instance>` |
| `SpiExt` | `pac::Spi0`, etc. | `spi(sck, mosi, miso, config, clocks, sim)` | `Spi<Instance>` |
| `I2cExt` | `pac::I2c0`, etc. | `i2c(sda, scl, freq, clocks, sim)` | `I2c<Instance>` |
| `PitExt` | `pac::Pit` | `split(sim, clocks)` | `PitChannels` |
| `FtmExt` | `pac::Ftm0`, etc. | `pwm(freq, clocks, sim)` | `FtmXChannels` |
| `AdcExt` | `pac::Adc0`, etc. | `adc(clocks, sim)` | `Adc<Instance>` |
| `DmaExt` | `pac::Dma` | `split(dmamux, sim)` | `DmaChannels` |
| `UsbBusExt` | `pac::Usb0` | `usb_bus(sim)` | `UsbBus` |

```rust
use mk20dx_hal::prelude::*;

let dp = pac::Peripherals::take().unwrap();
let cp = cortex_m::Peripherals::take().unwrap();

// Disable watchdog (consumes WDOG peripheral)
dp.WDOG.disable();

// Configure clocks: MCG → PLL → 72 MHz (consumes MCG + OSC, borrows SIM)
let clocks = dp.MCG.constrain().freeze(dp.OSC, &dp.SIM);

// Split GPIO ports (consumes PORT + GPIO peripherals, borrows SIM for clock gating)
let pins_a = dp.PORTA.split(dp.PTA, &dp.SIM);
let pins_c = dp.PORTC.split(dp.PTC, &dp.SIM);

// SysTick delay
let mut delay = Delay::new(cp.SYST, &clocks);

// Use pins with type-state
let mut led = pins_c.pc5.into_push_pull_output();
led.set_high().unwrap();
```

Once consumed via `constrain()`/`split()`/`disable()`, the PAC struct cannot be accessed again — prevents register aliasing.

### Type-State GPIO

Pin modes are encoded in the type system:
```rust
let led = pins_a.pa5.into_push_pull_output();  // Pin<'A', 5, Output<PushPull>>
led.set_high().unwrap();
let button = pins_b.pb3.into_pull_up_input();  // Pin<'B', 3, Input<PullUp>>
```

Mode transitions consume the pin and return a new type. Invalid operations are compile errors.

### Clocks Token

Most peripheral `::new()` methods require a `&Clocks` reference to prove clocks have been configured and to read bus frequencies for baud rate calculation.

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `embedded-hal` | 1.0 | Trait definitions (digital, SPI, I2C, delay, PWM) |
| `embedded-hal-nb` | 1.0 | Non-blocking serial traits |
| `embedded-io` | 0.6 | Blocking byte-stream I/O (UART) |
| `cortex-m` | 0.7 | Core peripheral access (NVIC, SysTick), critical sections |
| `cortex-m-rt` | 0.7 | Runtime (optional, via `rt` feature) |
| `nb` | 1.1 | Non-blocking result type |
| `fugit` | 0.3 | Duration and rate types |
| `usb-device` | 0.3 | USB device bus trait (`UsbBus`) |
| `mk20d5` / `mk20d7` | 0.1 | PAC (selected by feature flag) |

## Build Commands

```bash
# Check for Teensy 3.1/3.2 (default)
cargo check --features mk20d7 --target thumbv7em-none-eabi

# Check for Teensy 3.0 (must disable default mk20d7 feature)
cargo check --no-default-features --features mk20d5 --target thumbv7em-none-eabi

# Build with runtime support
cargo check --features "mk20d7,rt" --target thumbv7em-none-eabi
```

## embedded-hal Trait Map

| Module | Trait | HAL Type | Peripheral |
|--------|-------|----------|------------|
| `digital` | `InputPin` | `Pin<P, N, Input<PULL>>` | PORT + GPIO |
| `digital` | `OutputPin` | `Pin<P, N, Output<MODE>>` | PORT + GPIO |
| `digital` | `StatefulOutputPin` | `Pin<P, N, Output<MODE>>` | PORT + GPIO |
| `spi` | `SpiBus` | `Spi<SPI0>` | DSPI (SPI0, SPI1) |
| `i2c` | `I2c` | `I2c<I2C0>` | I2C (I2C0, I2C1) |
| `delay` | `DelayNs` | `Delay` | SysTick |
| `pwm` | `SetDutyCycle` | `PwmChannel<FTM, CH>` | FTM (FTM0, FTM1, FTM2) |
| `embedded-hal-nb` | `serial::Read` | `Serial<UART0>` | UART (UART0, UART1, UART2) |
| `embedded-hal-nb` | `serial::Write` | `Serial<UART0>` | UART (UART0, UART1, UART2) |
| HAL-specific | ADC read | `Adc<ADC0>` | ADC (ADC0, ADC1) |
| HAL-specific | PIT timer | `PitChannel<CH>` | PIT (4 channels) |
| HAL-specific | DMA transfer | `DmaChannel<CH>` | eDMA + DMAMUX (4/16 channels) |
| `usb-device` | `UsbBus` | `UsbBus` | USB0 (full-speed device) |

## Kinetis-Specific Notes

### PORT vs GPIO

Kinetis chips separate pin configuration into two peripheral blocks:
- **PORT** (PORTA..PORTE): Pin mux selection (3-bit MUX field), pull-up/down, drive strength, interrupt config
- **GPIO** (GPIOA..GPIOE): Data direction, data input/output, set/clear/toggle

The HAL's GPIO module must coordinate both. Pin mux must be set to ALT1 (GPIO mode) for digital I/O, or to the appropriate alternate function for UART/SPI/I2C/FTM.

The PAC now provides semantic MUX enum names (e.g., `mux().gpio()`, `mux().disabled()`) instead of raw bit patterns (`mux()._001()`), thanks to ergonomics patches.

### PAC Semantic Enums Available

The PAC's ergonomics patches provide named enums the HAL should use directly:
- **PORT PCR MUX**: `Disabled`, `Gpio`, `Alt2`..`Alt7` (pin function selection)
- **FTM SC CLKS**: `None`, `System`, `Fixed`, `External` (clock source)
- **FTM SC PS**: `Div1`, `Div2`, `Div4`..`Div128` (prescaler)
- **ADC CFG1**: `Mode8Bit`, `Mode12Bit`, `Mode10Bit`, `Mode16Bit`; clock source and sample time enums
- **MCG C1/C2/C6**: Clock source, FLL divider, oscillator range, PLL VDIV enums
- **SIM SOPT2**: PLL/FLL select, clock output select enums
- **DMA TCD ATTR**: Transfer size enums (`Bits8`, `Bits16`, `Bits32`, `Burst16`)
- **DMAMUX CHCFG SOURCE**: Peripheral source enums (`Disabled`, `Uart0rx`, `Spi0rx`, `Adc0`, `AlwaysOn0`, etc.)

### Clock Tree

The MK20 clock tree flows through:
1. **External crystal** (16 MHz on Teensy) → **MCG** (Multipurpose Clock Generator)
2. MCG configures FLL or PLL → produces **MCGOUTCLK**
3. **SIM** divides MCGOUTCLK into bus clocks: Core, Bus, FlexBus, Flash
4. **SIM SCGC registers** gate clocks to individual peripherals (must be enabled before access)

The `clock` module must configure MCG and SIM, then return a `Clocks` struct that records the resulting frequencies.

### embedded-hal 1.0 Notes

Traits removed from `embedded-hal` 1.0 that affect us:
- **ADC**: No standard trait — use HAL-specific API
- **Serial**: Moved to `embedded-hal-nb` (non-blocking) and `embedded-io` (blocking)
- **Timer/CountDown**: No standard trait — use HAL-specific API
- **CAN**: Moved to `embedded-can` crate
