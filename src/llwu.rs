//! Low-Leakage Wakeup Unit (LLWU) driver.
//!
//! The LLWU manages wakeup sources for LLS and VLLSx low-power modes.
//! In these modes, normal NVIC interrupts cannot wake the processor —
//! only LLWU-configured pin edges and internal module flags can trigger
//! a wakeup.
//!
//! # Wakeup Sources
//!
//! - **Pins**: 16 external pin inputs (LLWU_P0..P15) with configurable
//!   edge detection (rising, falling, or any change).
//! - **Modules**: 8 internal module wakeup sources (LPTMR, RTC alarm,
//!   CMP, etc.) that can wake from LLS/VLLSx.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::llwu::{Llwu, LlwuPin, WakeEdge, LlwuModule};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! let mut llwu = dp.llwu.llwu(&dp.sim);
//!
//! // Enable LLWU_P5 (PTA4) to wake on falling edge
//! llwu.enable_pin(LlwuPin::P5, WakeEdge::Falling);
//!
//! // Enable LPTMR module wakeup
//! llwu.enable_module(LlwuModule::Lptmr);
//!
//! // ... enter LLS or VLLS via PowerControl ...
//!
//! // After waking, check which source triggered
//! let flags = llwu.pin_flags();
//! llwu.clear_all_pin_flags();
//! ```

use crate::pac;

/// LLWU wakeup pin index (LLWU_P0..P15).
///
/// Pin mapping is chip-specific. On MK20DX:
/// - P0: PTE1, P1: PTE2, P2: PTE4, P3: PTA4
/// - P4: PTA13, P5: PTB0, P6: PTC1, P7: PTC3
/// - P8: PTC4, P9: PTC5, P10: PTC6, P11: PTC11
/// - P12: PTD0, P13: PTD2, P14: PTD4, P15: PTD6
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum LlwuPin {
    P0 = 0, P1 = 1, P2 = 2, P3 = 3,
    P4 = 4, P5 = 5, P6 = 6, P7 = 7,
    P8 = 8, P9 = 9, P10 = 10, P11 = 11,
    P12 = 12, P13 = 13, P14 = 14, P15 = 15,
}

/// Wakeup edge detection mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum WakeEdge {
    /// Wakeup on rising edge.
    Rising,
    /// Wakeup on falling edge.
    Falling,
    /// Wakeup on any edge (rising or falling).
    AnyEdge,
}

/// LLWU internal module wakeup source.
///
/// Module-to-bit mapping on MK20DX:
/// - WUME0: LPTMR
/// - WUME1: CMP0
/// - WUME2: CMP1 (mk20d7 only)
/// - WUME3: Reserved
/// - WUME4: Reserved
/// - WUME5: RTC Alarm
/// - WUME6: Reserved
/// - WUME7: RTC Seconds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LlwuModule {
    /// LPTMR (module 0).
    Lptmr,
    /// CMP0 (module 1).
    Cmp0,
    /// CMP1 (module 2, mk20d7 only).
    Cmp1,
    /// RTC Alarm (module 5).
    RtcAlarm,
    /// RTC Seconds (module 7).
    RtcSeconds,
}

/// Low-Leakage Wakeup Unit driver.
pub struct Llwu {
    _private: (),
}

/// Extension trait to initialize the LLWU from the PAC peripheral.
pub trait LlwuExt {
    /// Consume the LLWU peripheral and return a driver handle.
    fn llwu(self, sim: &pac::Sim) -> Llwu;
}

impl LlwuExt for pac::Llwu {
    fn llwu(self, _sim: &pac::Sim) -> Llwu {
        // LLWU has no clock gate — it's always accessible.
        Llwu { _private: () }
    }
}

impl Llwu {
    fn regs() -> &'static pac::llwu::RegisterBlock {
        unsafe { &*pac::Llwu::PTR }
    }

    /// Enable a pin as a wakeup source with the specified edge.
    pub fn enable_pin(&mut self, pin: LlwuPin, edge: WakeEdge) {
        let val = match edge {
            WakeEdge::Rising => 0b01,
            WakeEdge::Falling => 0b10,
            WakeEdge::AnyEdge => 0b11,
        };
        let pin_num = pin as u8;
        let reg_index = pin_num / 4; // PE1..PE4, 4 pins per register
        let field_shift = (pin_num % 4) * 2; // 2 bits per field

        let llwu = Self::regs();

        // Read-modify-write the appropriate PEx register
        // Use write() rather than modify() to avoid type mismatch in match arms
        match reg_index {
            0 => {
                let r = llwu.pe1().read().bits();
                llwu.pe1().write(|w| unsafe {
                    w.bits((r & !(0x3 << field_shift)) | (val << field_shift))
                });
            },
            1 => {
                let r = llwu.pe2().read().bits();
                llwu.pe2().write(|w| unsafe {
                    w.bits((r & !(0x3 << field_shift)) | (val << field_shift))
                });
            },
            2 => {
                let r = llwu.pe3().read().bits();
                llwu.pe3().write(|w| unsafe {
                    w.bits((r & !(0x3 << field_shift)) | (val << field_shift))
                });
            },
            3 => {
                let r = llwu.pe4().read().bits();
                llwu.pe4().write(|w| unsafe {
                    w.bits((r & !(0x3 << field_shift)) | (val << field_shift))
                });
            },
            _ => unreachable!(),
        }
    }

    /// Disable a pin as a wakeup source.
    pub fn disable_pin(&mut self, pin: LlwuPin) {
        let pin_num = pin as u8;
        let reg_index = pin_num / 4;
        let field_shift = (pin_num % 4) * 2;

        let llwu = Self::regs();

        match reg_index {
            0 => {
                let r = llwu.pe1().read().bits();
                llwu.pe1().write(|w| unsafe { w.bits(r & !(0x3 << field_shift)) });
            },
            1 => {
                let r = llwu.pe2().read().bits();
                llwu.pe2().write(|w| unsafe { w.bits(r & !(0x3 << field_shift)) });
            },
            2 => {
                let r = llwu.pe3().read().bits();
                llwu.pe3().write(|w| unsafe { w.bits(r & !(0x3 << field_shift)) });
            },
            3 => {
                let r = llwu.pe4().read().bits();
                llwu.pe4().write(|w| unsafe { w.bits(r & !(0x3 << field_shift)) });
            },
            _ => unreachable!(),
        }
    }

    /// Enable an internal module as a wakeup source.
    pub fn enable_module(&mut self, module: LlwuModule) {
        let llwu = Self::regs();
        let bit = module_bit(module);
        llwu.me().modify(|r, w| unsafe {
            w.bits(r.bits() | (1 << bit))
        });
    }

    /// Disable an internal module as a wakeup source.
    pub fn disable_module(&mut self, module: LlwuModule) {
        let llwu = Self::regs();
        let bit = module_bit(module);
        llwu.me().modify(|r, w| unsafe {
            w.bits(r.bits() & !(1 << bit))
        });
    }

    /// Read pin wakeup flags (16 bits, one per LLWU pin).
    ///
    /// Bit N is set if LLWU_PN triggered the wakeup.
    pub fn pin_flags(&self) -> u16 {
        let llwu = Self::regs();
        let f1 = llwu.f1().read().bits();
        let f2 = llwu.f2().read().bits();
        (f2 as u16) << 8 | f1 as u16
    }

    /// Check if a specific pin triggered the wakeup.
    pub fn pin_woke(&self, pin: LlwuPin) -> bool {
        let flags = self.pin_flags();
        flags & (1 << pin as u8) != 0
    }

    /// Clear a specific pin wakeup flag (write-1-to-clear).
    pub fn clear_pin_flag(&mut self, pin: LlwuPin) {
        let llwu = Self::regs();
        let pin_num = pin as u8;
        if pin_num < 8 {
            llwu.f1().write(|w| unsafe { w.bits(1 << pin_num) });
        } else {
            llwu.f2().write(|w| unsafe { w.bits(1 << (pin_num - 8)) });
        }
    }

    /// Clear all pin wakeup flags.
    pub fn clear_all_pin_flags(&mut self) {
        let llwu = Self::regs();
        llwu.f1().write(|w| unsafe { w.bits(0xFF) });
        llwu.f2().write(|w| unsafe { w.bits(0xFF) });
    }

    /// Read module wakeup flags (8 bits, one per module).
    ///
    /// These flags are read-only and cleared by clearing the source
    /// in the respective module (e.g., clear LPTMR TCF).
    pub fn module_flags(&self) -> u8 {
        Self::regs().f3().read().bits()
    }

    /// Check if a specific module triggered the wakeup.
    pub fn module_woke(&self, module: LlwuModule) -> bool {
        let bit = module_bit(module);
        self.module_flags() & (1 << bit) != 0
    }

    /// Release the LLWU peripheral, returning the PAC type.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers.
    pub unsafe fn release(self) -> pac::Llwu {
        pac::Llwu::steal()
    }
}

fn module_bit(module: LlwuModule) -> u8 {
    match module {
        LlwuModule::Lptmr => 0,
        LlwuModule::Cmp0 => 1,
        LlwuModule::Cmp1 => 2,
        LlwuModule::RtcAlarm => 5,
        LlwuModule::RtcSeconds => 7,
    }
}
