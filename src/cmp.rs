//! Analog comparator (CMP) driver.
//!
//! The MK20 has 2 comparators (CMP0, CMP1) on both variants, plus
//! CMP2 on the MK20DX256 (mk20d7). Each comparator can compare two
//! analog inputs (IN0-IN7) with optional hysteresis and an internal
//! 6-bit DAC reference.
//!
//! # SCR w1c Hazard
//!
//! The SCR register mixes write-1-to-clear flags (CFF bit 1, CFR bit 2)
//! with read/write config bits (IEF bit 3, IER bit 4, DMAEN bit 6).
//! Using `modify()` would read back the flag bits as 1 and write them
//! back, accidentally clearing them. All SCR writes in this driver use
//! `write()` with manual config bit preservation.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::cmp::{Input, CmpDacVref};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... disable watchdog, configure clocks ...
//! let mut cmp = dp.cmp0.cmp(Input::IN0, Input::IN1, &dp.sim);
//!
//! // Use internal 6-bit DAC as minus input
//! cmp.set_minus_input(Input::INTERNAL_DAC);
//! cmp.set_internal_dac(32, CmpDacVref::Vin1); // mid-scale
//!
//! let result = cmp.output(); // true if plus > minus
//! ```

use core::marker::PhantomData;

use crate::pac;

// ----- Configuration Types -----

/// Comparator input selection (IN0-IN7).
///
/// Input 7 is typically connected to the CMP internal 6-bit DAC output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input(pub u8);

impl Input {
    pub const IN0: Input = Input(0);
    pub const IN1: Input = Input(1);
    pub const IN2: Input = Input(2);
    pub const IN3: Input = Input(3);
    pub const IN4: Input = Input(4);
    pub const IN5: Input = Input(5);
    pub const IN6: Input = Input(6);
    pub const IN7: Input = Input(7);
    /// Alias for IN7 — the CMP internal 6-bit DAC output.
    pub const INTERNAL_DAC: Input = Input(7);
}

/// Voltage reference source for the CMP internal 6-bit DAC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpDacVref {
    /// Vin1 (typically VDDA / 3.3V).
    Vin1,
    /// Vin2 (typically VREF_OUT).
    Vin2,
}

/// Comparator hysteresis level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hysteresis {
    /// No hysteresis.
    Level0,
    /// Level 1 hysteresis.
    Level1,
    /// Level 2 hysteresis.
    Level2,
    /// Level 3 hysteresis (maximum).
    Level3,
}

// ----- Instance Markers -----

/// Marker type for CMP0.
pub struct Cmp0;

/// Marker type for CMP1.
pub struct Cmp1;

/// Marker type for CMP2 (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Cmp2;

// ----- Driver Type -----

/// Analog comparator driver.
pub struct Cmp<INST> {
    _inst: PhantomData<INST>,
}

// ----- Extension Trait -----

/// Extension trait for creating comparator drivers from PAC CMP peripherals.
pub trait CmpExt: Sized {
    type Instance;

    /// Enable the CMP clock gate, configure inputs, and enable the comparator.
    ///
    /// Defaults: low-speed mode, no hysteresis, no filter, no interrupts,
    /// internal DAC disabled.
    fn cmp(self, plus: Input, minus: Input, sim: &pac::Sim) -> Cmp<Self::Instance>;
}

// ----- SCR w1c-safe helper -----

/// Read the current SCR config bits (IEF, IER, DMAEN), with w1c flags
/// masked off, then apply a modification and write back safely.
///
/// The closure receives the current config value (with CFF=0, CFR=0)
/// and should return the desired value to write.
macro_rules! scr_safe_write {
    ($cmp:expr, $modify:expr) => {{
        let scr = $cmp.scr().read();
        let mut val: u8 = 0;
        if scr.ief().is_1() {
            val |= 1 << 3;
        }
        if scr.ier().is_1() {
            val |= 1 << 4;
        }
        if scr.dmaen().is_1() {
            val |= 1 << 6;
        }
        // CFF (bit 1) and CFR (bit 2) are left as 0 — safe for w1c
        let modify_fn: fn(u8) -> u8 = $modify;
        val = modify_fn(val);
        $cmp.scr().write(|w| unsafe { w.bits(val) });
    }};
}

// ----- Per-instance macro -----

macro_rules! cmp_impl {
    ($PacType:ty, $Instance:ty) => {
        impl Cmp<$Instance> {
            fn regs() -> &'static <$PacType as core::ops::Deref>::Target {
                unsafe { &*<$PacType>::PTR }
            }

            fn init(plus: Input, minus: Input) -> Self {
                let cmp = Self::regs();

                // CR0: no hysteresis, no filter
                cmp.cr0().write(|w| w.hystctr()._00().filter_cnt()._000());

                // MUXCR: set input channels
                cmp.muxcr().write(|w| unsafe {
                    w.psel().bits(plus.0 & 0x07)
                     .msel().bits(minus.0 & 0x07)
                });

                // DACCR: internal DAC disabled
                cmp.daccr().write(|w| w.dacen()._0());

                // SCR: clear flags, no interrupts, no DMA
                cmp.scr().write(|w| unsafe { w.bits(0) });

                // CR1: enable comparator, low-speed, no invert
                cmp.cr1().write(|w| w.en()._1());

                Cmp { _inst: PhantomData }
            }

            /// Read the current comparator output.
            ///
            /// Returns `true` if the plus input is greater than the minus input
            /// (or the inverse if `set_inverted(true)` was called).
            pub fn output(&self) -> bool {
                Self::regs().scr().read().cout().bit()
            }

            /// Set the plus (non-inverting) input.
            pub fn set_plus_input(&mut self, input: Input) {
                Self::regs()
                    .muxcr()
                    .modify(|_, w| unsafe { w.psel().bits(input.0 & 0x07) });
            }

            /// Set the minus (inverting) input.
            pub fn set_minus_input(&mut self, input: Input) {
                Self::regs()
                    .muxcr()
                    .modify(|_, w| unsafe { w.msel().bits(input.0 & 0x07) });
            }

            /// Configure the CMP internal 6-bit DAC.
            ///
            /// `level` is a 6-bit value (0-63) that sets the DAC output voltage:
            /// `Vout = Vin * (level + 1) / 64`
            ///
            /// Values above 63 are silently masked.
            pub fn set_internal_dac(&mut self, level: u8, vref: CmpDacVref) {
                let cmp = Self::regs();
                cmp.daccr().write(|w| unsafe {
                    let w = w.dacen()._1().vosel().bits(level & 0x3F);
                    match vref {
                        CmpDacVref::Vin1 => w.vrsel()._0(),
                        CmpDacVref::Vin2 => w.vrsel()._1(),
                    }
                });
            }

            /// Disable the CMP internal 6-bit DAC.
            pub fn disable_internal_dac(&mut self) {
                Self::regs().daccr().write(|w| w.dacen()._0());
            }

            /// Set the hysteresis level.
            pub fn set_hysteresis(&mut self, level: Hysteresis) {
                Self::regs().cr0().modify(|_, w| match level {
                    Hysteresis::Level0 => w.hystctr()._00(),
                    Hysteresis::Level1 => w.hystctr()._01(),
                    Hysteresis::Level2 => w.hystctr()._10(),
                    Hysteresis::Level3 => w.hystctr()._11(),
                });
            }

            /// Set whether the comparator output is inverted.
            pub fn set_inverted(&mut self, inverted: bool) {
                Self::regs().cr1().modify(|_, w| if inverted {
                    w.inv()._1()
                } else {
                    w.inv()._0()
                });
            }

            /// Enable the comparator (CR1.EN = 1).
            pub fn enable(&mut self) {
                Self::regs().cr1().modify(|_, w| w.en()._1());
            }

            /// Disable the comparator (CR1.EN = 0).
            pub fn disable(&mut self) {
                Self::regs().cr1().modify(|_, w| w.en()._0());
            }

            /// Returns `true` if a rising edge has been detected on COUT.
            pub fn rising_edge(&self) -> bool {
                Self::regs().scr().read().cfr().is_1()
            }

            /// Returns `true` if a falling edge has been detected on COUT.
            pub fn falling_edge(&self) -> bool {
                Self::regs().scr().read().cff().is_1()
            }

            /// Clear both edge flags (CFR and CFF) using a w1c-safe write.
            pub fn clear_flags(&mut self) {
                let cmp = Self::regs();
                // Read config bits, write back with CFF=1 and CFR=1 to clear them
                let scr = cmp.scr().read();
                let mut val: u8 = 0;
                if scr.ief().is_1() {
                    val |= 1 << 3;
                }
                if scr.ier().is_1() {
                    val |= 1 << 4;
                }
                if scr.dmaen().is_1() {
                    val |= 1 << 6;
                }
                // Set CFF (bit 1) and CFR (bit 2) to clear them (w1c)
                val |= (1 << 1) | (1 << 2);
                cmp.scr().write(|w| unsafe { w.bits(val) });
            }

            /// Enable the rising edge interrupt (IER) using a w1c-safe write.
            pub fn enable_rising_interrupt(&mut self) {
                scr_safe_write!(Self::regs(), |val| val | (1 << 4));
            }

            /// Enable the falling edge interrupt (IEF) using a w1c-safe write.
            pub fn enable_falling_interrupt(&mut self) {
                scr_safe_write!(Self::regs(), |val| val | (1 << 3));
            }

            /// Disable both edge interrupts (IER and IEF) using a w1c-safe write.
            pub fn disable_interrupts(&mut self) {
                scr_safe_write!(Self::regs(), |val| val & !(1 << 3) & !(1 << 4));
            }
        }

        impl CmpExt for $PacType {
            type Instance = $Instance;

            fn cmp(self, plus: Input, minus: Input, sim: &pac::Sim) -> Cmp<$Instance> {
                // Enable CMP clock gate (shared for all instances)
                sim.scgc4().modify(|_, w| w.cmp().enabled());
                Cmp::<$Instance>::init(plus, minus)
            }
        }
    };
}

// Both variants have CMP0 and CMP1
cmp_impl!(pac::Cmp0, Cmp0);
cmp_impl!(pac::Cmp1, Cmp1);

// Only mk20d7 has CMP2
#[cfg(feature = "mk20d7")]
cmp_impl!(pac::Cmp2, Cmp2);
