# mk20dx-hal

Hardware Abstraction Layer for the NXP Kinetis MK20DX128 and MK20DX256
microcontrollers, implementing [`embedded-hal`] 1.0 traits on top of the
[`mk20dx-pac`] peripheral access crates.

[`embedded-hal`]: https://crates.io/crates/embedded-hal
[`mk20dx-pac`]: ../mk20dx-pac/

## Supported Hardware

| Feature | Chip | Board | Core | Clock | RAM | Flash | DMA |
|---------|------|-------|------|-------|-----|-------|-----|
| `mk20d5` | MK20DX128VLH5 | Teensy 3.0 | Cortex-M4 | 48 MHz | 16 KB | 128 KB | 4 ch |
| `mk20d7` (default) | MK20DX256VLH7 | Teensy 3.1/3.2 | Cortex-M4 | 72 MHz | 64 KB | 256 KB | 16 ch |

Target: `thumbv7em-none-eabi` (Cortex-M4, no FPU).

## Usage

Add the HAL to your `Cargo.toml` with the appropriate feature for your board:

```toml
[dependencies]
mk20dx-hal = { path = "../mk20dx-hal", features = ["mk20d7"] }  # Teensy 3.1/3.2
# mk20dx-hal = { path = "../mk20dx-hal", features = ["mk20d5"] }  # Teensy 3.0
```

Copy `.cargo/config.toml` and the appropriate `memory.x` linker script into
your project. The target is set in `.cargo/config.toml` so `--target` is not
needed on the command line.

> **Flash security warning:** The 16-byte flash configuration field at 0x400
> must have FSEC = `0xFE` (unsecured). If erased to `0xFF`, the chip becomes
> **permanently secured** and cannot be reflashed without a mass erase recovery.
> The HAL's `flash_config.rs` handles this automatically.

### Minimal Example

```rust
#![no_std]
#![no_main]

use mk20dx_hal::prelude::*;
use mk20dx_hal::{pac, delay::Delay};

#[cortex_m_rt::entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();
    let cp = cortex_m::Peripherals::take().unwrap();

    dp.wdog.disable();
    let clocks = dp.mcg.constrain().freeze(dp.osc, &dp.sim);

    let pins_c = dp.portc.split(dp.ptc, &dp.sim);
    let mut led = pins_c.pc5.into_push_pull_output();

    let mut delay = Delay::new(cp.SYST, &clocks);
    loop {
        led.set_high().unwrap();
        delay.delay_ms(500);
        led.set_low().unwrap();
        delay.delay_ms(500);
    }
}
```

## Peripherals

| Peripheral | Module | embedded-hal Trait | Notes |
|------------|--------|--------------------|-------|
| GPIO | `gpio` | `InputPin`, `OutputPin`, `StatefulOutputPin` | Type-state pin modes, 5 ports (A-E) |
| UART | `uart` | `serial::Read`, `serial::Write`, `embedded-io` | UART0-2, FIFO, DMA |
| SPI | `spi` | `SpiBus` | DSPI, baud rate calc, DMA (PUSHR) |
| I2C | `i2c` | `I2c` | 7-bit addressing, I2C0 (+I2C1 on mk20d7) |
| PWM | `pwm` | `SetDutyCycle` | FTM0-2, combined mode, dead-time, input capture, output compare, quadrature decoder |
| ADC | `adc` | *(HAL-specific)* | Calibration, DMA, PDB-triggered multi-channel scan |
| PIT | `timer` | *(HAL-specific)* | 4-channel periodic interrupt timer |
| DMA | `dma` | *(HAL-specific)* | eDMA + DMAMUX, channel linking, scatter-gather |
| USB | `usb` | `usb-device::UsbBus` | Full-speed device, ping-pong buffers |
| DAC | `dac` | *(HAL-specific)* | 12-bit, mk20d7 only |
| Flash | `flash` | `NorFlash`, `ReadNorFlash` | FTFL, 2 KB sectors |
| EEPROM | `eeprom` | *(HAL-specific)* | FlexMemory emulated EEPROM |
| CMP | `cmp` | *(HAL-specific)* | 3 comparators, internal 6-bit DAC |
| RTC | `rtc` | *(HAL-specific)* | 32.768 kHz, alarm, seconds counter |
| LPTMR | `lptmr` | *(HAL-specific)* | Low-power timer, multiple clock sources |
| CRC | `crc_module` | *(HAL-specific)* | Hardware CRC-16/CRC-32 |
| PDB | `pdb` | *(HAL-specific)* | Programmable delay block for ADC triggering |
| Clocks | `clocks` | | MCG PLL config, 72/96/120 MHz presets (mk20d7) |
| Delay | `delay` | `DelayNs` | SysTick-based |
| Power | `power` | *(HAL-specific)* | Wait, Stop, VLPR, VLPS, LLS, VLLS modes |
| LLWU | `llwu` | *(HAL-specific)* | Low-leakage wakeup unit |
| Watchdog | `watchdog` | | Disable only (20 bus-cycle unlock window) |

## Feature Flags

| Feature | Description |
|---------|-------------|
| `mk20d7` | MK20DX256 / Teensy 3.1/3.2 (default) |
| `mk20d5` | MK20DX128 / Teensy 3.0 |
| `rt` | Cortex-M runtime (`cortex-m-rt`) with interrupt vectors |
| `defmt` | `defmt::Format` derives on error and status types |
| `async` | Async support via `embassy-sync` + `embedded-hal-async` |
| `critical-section` | Forward `critical-section` feature to the PAC |

Features `mk20d5` and `mk20d7` are **mutually exclusive** — selecting both
is a compile error.

## Building

```bash
cargo check                                              # Teensy 3.1/3.2 (default)
cargo check --no-default-features --features mk20d5      # Teensy 3.0
cargo check --features rt                                 # With runtime support
cargo check --features async                              # With async support
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
