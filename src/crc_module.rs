//! Hardware CRC (Cyclic Redundancy Check) accelerator.
//!
//! The CRC module provides hardware-accelerated CRC calculation with
//! configurable polynomial, seed, bit width (16 or 32 bit), and
//! bit/byte transpose options.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::crc_module::{CrcConfig, CrcWidth};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... disable watchdog, configure clocks ...
//! let mut crc = dp.crc.crc_engine(&dp.sim);
//!
//! // Default: CRC-16-CCITT (polynomial 0x1021, seed 0xFFFF)
//! crc.configure(CrcConfig::crc16_ccitt());
//! crc.feed(&[0x01, 0x02, 0x03, 0x04]);
//! let result = crc.result();
//! ```

use crate::pac;

/// CRC bit width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CrcWidth {
    /// 16-bit CRC.
    Bits16,
    /// 32-bit CRC.
    Bits32,
}

/// Transpose mode for CRC data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Transpose {
    /// No transposition.
    None,
    /// Transpose bits within each byte.
    BitsInBytes,
    /// Transpose both bits and bytes.
    BitsAndBytes,
    /// Transpose bytes only.
    BytesOnly,
}

/// CRC configuration.
#[derive(Clone, Copy, Debug)]
pub struct CrcConfig {
    /// CRC polynomial.
    pub polynomial: u32,
    /// Initial seed value.
    pub seed: u32,
    /// CRC bit width (16 or 32).
    pub width: CrcWidth,
    /// Complement (XOR) the final result.
    pub complement_result: bool,
    /// Transpose mode for write data.
    pub write_transpose: Transpose,
    /// Transpose mode for read result.
    pub read_transpose: Transpose,
}

impl CrcConfig {
    /// CRC-16-CCITT: polynomial 0x1021, seed 0xFFFF, no transposition.
    pub const fn crc16_ccitt() -> Self {
        CrcConfig {
            polynomial: 0x1021,
            seed: 0xFFFF,
            width: CrcWidth::Bits16,
            complement_result: false,
            write_transpose: Transpose::None,
            read_transpose: Transpose::None,
        }
    }

    /// CRC-32 (Ethernet/ZIP): polynomial 0x04C11DB7, seed 0xFFFFFFFF,
    /// bit-reversed input/output, complement result.
    pub const fn crc32() -> Self {
        CrcConfig {
            polynomial: 0x04C11DB7,
            seed: 0xFFFFFFFF,
            width: CrcWidth::Bits32,
            complement_result: true,
            write_transpose: Transpose::BitsInBytes,
            read_transpose: Transpose::BitsAndBytes,
        }
    }
}

/// Hardware CRC engine driver.
pub struct Crc {
    _private: (),
}

/// Extension trait to initialize the CRC engine from the PAC peripheral.
pub trait CrcExt {
    /// Enable the CRC clock gate and return a driver handle.
    fn crc_engine(self, sim: &pac::Sim) -> Crc;
}

impl CrcExt for pac::Crc {
    fn crc_engine(self, sim: &pac::Sim) -> Crc {
        // Enable CRC clock gate (SIM SCGC6)
        sim.scgc6().modify(|_, w| w.crc().enabled());

        Crc { _private: () }
    }
}

impl Crc {
    fn regs() -> &'static pac::crc::RegisterBlock {
        unsafe { &*pac::Crc::PTR }
    }

    /// Configure the CRC engine with the given parameters.
    ///
    /// Writes the polynomial, seed, and control settings. After calling
    /// this, the engine is ready to accept data via [`feed`](Crc::feed).
    pub fn configure(&mut self, config: CrcConfig) {
        let crc = Self::regs();

        // Configure control register
        crc.ctrl().write(|w| {
            let w = match config.width {
                CrcWidth::Bits16 => w.tcrc()._0(),
                CrcWidth::Bits32 => w.tcrc()._1(),
            };
            let w = if config.complement_result {
                w.fxor()._1()
            } else {
                w.fxor()._0()
            };
            let w = match config.write_transpose {
                Transpose::None => w.tot()._00(),
                Transpose::BitsInBytes => w.tot()._01(),
                Transpose::BitsAndBytes => w.tot()._10(),
                Transpose::BytesOnly => w.tot()._11(),
            };
            let w = match config.read_transpose {
                Transpose::None => w.totr()._00(),
                Transpose::BitsInBytes => w.totr()._01(),
                Transpose::BitsAndBytes => w.totr()._10(),
                Transpose::BytesOnly => w.totr()._11(),
            };
            // Set WAS=1 to write seed
            w.was()._1()
        });

        // Write polynomial
        crc.crc_gpoly().write(|w| unsafe {
            w.low().bits(config.polynomial as u16)
             .high().bits((config.polynomial >> 16) as u16)
        });

        // Write seed value (WAS=1 means writes to DATA go to seed)
        crc.crc_crc().write(|w| unsafe {
            w.ll().bits(config.seed as u8)
             .lu().bits((config.seed >> 8) as u8)
             .hl().bits((config.seed >> 16) as u8)
             .hu().bits((config.seed >> 24) as u8)
        });

        // Switch back to data mode (WAS=0)
        crc.ctrl().modify(|_, w| w.was()._0());
    }

    /// Feed data bytes into the CRC engine.
    ///
    /// For best performance, write 32-bit words when data is 4-byte aligned.
    /// The hardware processes data as it's written.
    pub fn feed(&mut self, data: &[u8]) {
        let crc = Self::regs();

        // Write bytes one at a time (simplest, always correct)
        for &byte in data {
            crc.crc_crcll().write(|w| unsafe { w.crcll().bits(byte) });
        }
    }

    /// Read the current CRC result.
    ///
    /// For 16-bit CRC, only the lower 16 bits are meaningful.
    pub fn result(&self) -> u32 {
        let crc = Self::regs();
        let r = crc.crc_crc().read();
        (r.hu().bits() as u32) << 24
            | (r.hl().bits() as u32) << 16
            | (r.lu().bits() as u32) << 8
            | r.ll().bits() as u32
    }

    /// Read the 16-bit CRC result (for CRC-16 configurations).
    pub fn result_u16(&self) -> u16 {
        self.result() as u16
    }

    /// Reset the CRC to the seed value by re-writing it.
    ///
    /// Call [`configure`](Crc::configure) first to set up the seed.
    pub fn reset(&mut self, seed: u32) {
        let crc = Self::regs();

        // Switch to seed mode
        crc.ctrl().modify(|_, w| w.was()._1());

        // Write seed
        crc.crc_crc().write(|w| unsafe {
            w.ll().bits(seed as u8)
             .lu().bits((seed >> 8) as u8)
             .hl().bits((seed >> 16) as u8)
             .hu().bits((seed >> 24) as u8)
        });

        // Switch back to data mode
        crc.ctrl().modify(|_, w| w.was()._0());
    }

    /// Release the CRC peripheral, returning the PAC type.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers.
    pub unsafe fn release(self) -> pac::Crc {
        pac::Crc::steal()
    }
}
