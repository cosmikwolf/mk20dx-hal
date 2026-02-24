#![no_std]

// Ensure exactly one chip variant is selected
#[cfg(all(feature = "mk20d5", feature = "mk20d7"))]
compile_error!("Features `mk20d5` and `mk20d7` are mutually exclusive. Select only one.");

#[cfg(not(any(feature = "mk20d5", feature = "mk20d7")))]
compile_error!("Either feature `mk20d5` or `mk20d7` must be enabled.");

// Re-export the selected PAC
#[cfg(feature = "mk20d5")]
pub use mk20d5 as pac;
#[cfg(feature = "mk20d7")]
pub use mk20d7 as pac;

pub mod adc;
pub mod clocks;
pub mod cmp;
pub mod crc_module;
#[cfg(feature = "mk20d7")]
pub mod dac;
pub mod delay;
pub mod dma;
pub mod eeprom;
pub mod flash;
pub mod flash_config;
pub mod gpio;
pub mod i2c;
// No feature gate needed — PDB0 exists on both mk20d5 and mk20d7
pub mod llwu;
pub mod lptmr;
pub mod power;
pub mod pdb;
pub mod prelude;
pub mod pwm;
pub mod rtc;
pub mod spi;
pub mod time;
pub mod timer;
pub mod uart;
pub mod usb;
pub mod watchdog;

// Re-export cortex-m for convenience
pub use cortex_m;
