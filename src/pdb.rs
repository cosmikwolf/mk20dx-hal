//! PDB (Programmable Delay Block) driver.
//!
//! The PDB generates timed trigger outputs for ADC conversions. It provides
//! a single counter with programmable modulus and up to 2 pre-trigger outputs
//! per ADC channel, enabling precise timing of ADC conversion sequences.
//!
//! - MK20D5 (Teensy 3.0): 1 ADC channel (channel 0 → ADC0)
//! - MK20D7 (Teensy 3.1/3.2): 2 ADC channels (channel 0 → ADC0, channel 1 → ADC1)
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::pdb::{Prescaler, Multiplier, TriggerSource};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... configure clocks, disable watchdog ...
//! let mut pdb = dp.PDB0.constrain(&dp.SIM);
//!
//! // Configure PDB: software trigger, prescaler /1, mult x1, modulus 1000
//! pdb.configure(TriggerSource::Software, Prescaler::Div1, Multiplier::Mult1, 1000);
//! pdb.set_continuous(true);
//!
//! // Enable pre-trigger 0 on channel 0 with delay of 0
//! pdb.set_pretrigger_delay(0, 0, 0);
//! pdb.enable_pretrigger(0, 0);
//! pdb.load_ok();
//! pdb.enable();
//! pdb.software_trigger();
//! ```

use crate::pac;

/// Number of PDB ADC channels available on this variant.
#[cfg(feature = "mk20d5")]
const NUM_CHANNELS: u8 = 1;
#[cfg(feature = "mk20d7")]
const NUM_CHANNELS: u8 = 2;

/// Access the PDB0 register block.
fn regs() -> &'static pac::pdb0::RegisterBlock {
    // SAFETY: PTR is a valid pointer to the PDB0 register block.
    unsafe { &*pac::Pdb0::PTR }
}

// ----- Configuration Enums -----

/// PDB trigger source selection (SC.TRGSEL[3:0]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TriggerSource {
    Trigger0 = 0,
    Trigger1 = 1,
    Trigger2 = 2,
    Trigger3 = 3,
    Trigger4 = 4,
    Trigger5 = 5,
    Trigger6 = 6,
    Trigger7 = 7,
    Trigger8 = 8,
    Trigger9 = 9,
    Trigger10 = 10,
    Trigger11 = 11,
    Trigger12 = 12,
    Trigger13 = 13,
    Trigger14 = 14,
    /// Software trigger (SC.SWTRIG).
    Software = 15,
}

/// PDB prescaler divider (SC.PRESCALER[2:0]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Prescaler {
    Div1 = 0,
    Div2 = 1,
    Div4 = 2,
    Div8 = 3,
    Div16 = 4,
    Div32 = 5,
    Div64 = 6,
    Div128 = 7,
}

/// PDB prescaler multiplication factor (SC.MULT[1:0]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Multiplier {
    Mult1 = 0,
    Mult10 = 1,
    Mult20 = 2,
    Mult40 = 3,
}

// ----- PDB Driver -----

/// PDB driver.
///
/// Provides control over the Programmable Delay Block counter and
/// pre-trigger outputs for ADC hardware triggering.
pub struct Pdb {
    _private: (),
}

// ----- Channel register access helpers -----
//
// The mk20d5 PAC has non-array accessors (chc1(), chs(), chdly0(), chdly1())
// while mk20d7 has array-indexed accessors (chc1(n), chs(n), etc.).
// These helpers abstract the difference.

#[cfg(feature = "mk20d7")]
fn ch_c1(pdb: &pac::pdb0::RegisterBlock, ch: u8) -> &pac::pdb0::Chc1 {
    pdb.chc1(ch as usize)
}
#[cfg(feature = "mk20d5")]
fn ch_c1(pdb: &pac::pdb0::RegisterBlock, _ch: u8) -> &pac::pdb0::Chc1 {
    pdb.chc1()
}

#[cfg(feature = "mk20d7")]
fn ch_s(pdb: &pac::pdb0::RegisterBlock, ch: u8) -> &pac::pdb0::Chs {
    pdb.chs(ch as usize)
}
#[cfg(feature = "mk20d5")]
fn ch_s(pdb: &pac::pdb0::RegisterBlock, _ch: u8) -> &pac::pdb0::Chs {
    pdb.chs()
}

#[cfg(feature = "mk20d7")]
fn ch_dly0(pdb: &pac::pdb0::RegisterBlock, ch: u8) -> &pac::pdb0::Chdly0 {
    pdb.chdly0(ch as usize)
}
#[cfg(feature = "mk20d5")]
fn ch_dly0(pdb: &pac::pdb0::RegisterBlock, _ch: u8) -> &pac::pdb0::Chdly0 {
    pdb.chdly0()
}

#[cfg(feature = "mk20d7")]
fn ch_dly1(pdb: &pac::pdb0::RegisterBlock, ch: u8) -> &pac::pdb0::Chdly1 {
    pdb.chdly1(ch as usize)
}
#[cfg(feature = "mk20d5")]
fn ch_dly1(pdb: &pac::pdb0::RegisterBlock, _ch: u8) -> &pac::pdb0::Chdly1 {
    pdb.chdly1()
}

impl Pdb {
    /// Configure counter period, trigger source, prescaler, and multiplier.
    ///
    /// Also sets LDMOD=0 (immediate load on LDOK) and clears any pending
    /// interrupt flag. Does not enable the PDB — call [`enable`](Pdb::enable)
    /// afterward.
    pub fn configure(
        &mut self,
        trigger: TriggerSource,
        prescaler: Prescaler,
        multiplier: Multiplier,
        modulus: u16,
    ) {
        let pdb = regs();

        // Write SC: set trigger, prescaler, multiplier, LDMOD=0 (immediate)
        // SAFETY: trgsel (4-bit), prescaler (3-bit), mult (2-bit), ldmod (2-bit)
        // all within valid ranges via repr(u8) enums.
        pdb.sc().write(|w| unsafe {
            w.trgsel().bits(trigger as u8)
             .prescaler().bits(prescaler as u8)
             .mult().bits(multiplier as u8)
             .ldmod().bits(0)
        });

        // Set modulus
        // SAFETY: mod_ is a 16-bit field.
        pdb.mod_().write(|w| unsafe { w.mod_().bits(modulus) });
    }

    /// Enable or disable continuous mode.
    ///
    /// When enabled, the counter restarts from zero after reaching the
    /// modulus value. When disabled, the counter runs once (one-shot).
    pub fn set_continuous(&mut self, enabled: bool) {
        let pdb = regs();
        if enabled {
            pdb.sc().modify(|_, w| w.cont()._1());
        } else {
            pdb.sc().modify(|_, w| w.cont()._0());
        }
    }

    /// Set the pre-trigger delay for a channel/pre-trigger pair.
    ///
    /// `channel`: 0 on mk20d5, 0-1 on mk20d7.
    /// `pretrigger`: 0 or 1.
    /// `delay_ticks`: counter value at which the pre-trigger fires.
    ///
    /// Call [`load_ok`](Pdb::load_ok) after setting delays — delay registers
    /// are buffered and only take effect after LDOK is set.
    pub fn set_pretrigger_delay(&mut self, channel: u8, pretrigger: u8, delay_ticks: u16) {
        debug_assert!(channel < NUM_CHANNELS);
        debug_assert!(pretrigger <= 1);
        let pdb = regs();

        // SAFETY: dly is a 16-bit field.
        if pretrigger == 0 {
            ch_dly0(pdb, channel).write(|w| unsafe { w.dly().bits(delay_ticks) });
        } else {
            ch_dly1(pdb, channel).write(|w| unsafe { w.dly().bits(delay_ticks) });
        }
    }

    /// Enable a pre-trigger output.
    ///
    /// Sets the EN bit and TOS bit (use PDB delay, not bypass) for the
    /// given pre-trigger.
    pub fn enable_pretrigger(&mut self, channel: u8, pretrigger: u8) {
        debug_assert!(channel < NUM_CHANNELS);
        debug_assert!(pretrigger <= 1);
        let pdb = regs();
        let bit = 1u8 << pretrigger;

        ch_c1(pdb, channel).modify(|r, w| {
            let en = r.en().bits() | bit;
            let tos = r.tos().bits() | bit;
            // SAFETY: en and tos are 8-bit fields; we only set valid pre-trigger bits.
            unsafe { w.en().bits(en).tos().bits(tos) }
        });
    }

    /// Disable a pre-trigger output.
    pub fn disable_pretrigger(&mut self, channel: u8, pretrigger: u8) {
        debug_assert!(channel < NUM_CHANNELS);
        debug_assert!(pretrigger <= 1);
        let pdb = regs();
        let bit = 1u8 << pretrigger;

        ch_c1(pdb, channel).modify(|r, w| {
            let en = r.en().bits() & !bit;
            let tos = r.tos().bits() & !bit;
            // SAFETY: en and tos are 8-bit fields.
            unsafe { w.en().bits(en).tos().bits(tos) }
        });
    }

    /// Enable back-to-back mode for a pre-trigger.
    ///
    /// When enabled, the pre-trigger fires when the previous ADC conversion
    /// completes, instead of waiting for a PDB delay match. This is used to
    /// chain multiple ADC conversions without gaps.
    pub fn enable_back_to_back(&mut self, channel: u8, pretrigger: u8) {
        debug_assert!(channel < NUM_CHANNELS);
        debug_assert!(pretrigger <= 1);
        let pdb = regs();
        let bit = 1u8 << pretrigger;

        ch_c1(pdb, channel).modify(|r, w| {
            let bb = r.bb().bits() | bit;
            // SAFETY: bb is an 8-bit field.
            unsafe { w.bb().bits(bb) }
        });
    }

    /// Disable back-to-back mode for a pre-trigger.
    pub fn disable_back_to_back(&mut self, channel: u8, pretrigger: u8) {
        debug_assert!(channel < NUM_CHANNELS);
        debug_assert!(pretrigger <= 1);
        let pdb = regs();
        let bit = 1u8 << pretrigger;

        ch_c1(pdb, channel).modify(|r, w| {
            let bb = r.bb().bits() & !bit;
            // SAFETY: bb is an 8-bit field.
            unsafe { w.bb().bits(bb) }
        });
    }

    /// Load buffered register values (sets LDOK).
    ///
    /// Must be called after writing MOD or CHnDLYm registers. Buffered
    /// values take effect based on the LDMOD setting (default: immediately).
    pub fn load_ok(&mut self) {
        regs().sc().modify(|_, w| w.ldok().set_bit());
    }

    /// Enable the PDB counter (sets PDBEN).
    pub fn enable(&mut self) {
        regs().sc().modify(|_, w| w.pdben()._1());
    }

    /// Disable the PDB counter (clears PDBEN).
    pub fn disable(&mut self) {
        regs().sc().modify(|_, w| w.pdben()._0());
    }

    /// Issue a software trigger.
    ///
    /// If TRGSEL=15 (software trigger), this starts the PDB counter.
    /// In continuous mode, only the first trigger is needed.
    pub fn software_trigger(&mut self) {
        regs().sc().modify(|_, w| w.swtrig().set_bit());
    }

    /// Read the current counter value.
    pub fn counter(&self) -> u16 {
        regs().cnt().read().cnt().bits()
    }

    /// Read and clear sequence error flags for a channel.
    ///
    /// Returns the error flags (one bit per pre-trigger). Clears all
    /// error flags for the channel.
    pub fn clear_errors(&mut self, channel: u8) -> u8 {
        debug_assert!(channel < NUM_CHANNELS);
        let pdb = regs();
        let err = ch_s(pdb, channel).read().err().bits();
        if err != 0 {
            // Write 0 to ERR bits to clear (w1c — no, ERR is w0c: write 0 to clear)
            // SAFETY: Writing 0 to the err field clears the error flags.
            ch_s(pdb, channel).write(|w| unsafe { w.err().bits(0) });
        }
        err
    }

    /// Release the PDB peripheral.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers.
    pub unsafe fn release(self) -> pac::Pdb0 {
        pac::Pdb0::steal()
    }
}

// ----- Extension Trait -----

/// Extension trait for creating the PDB driver from the PAC peripheral.
pub trait PdbExt: Sized {
    /// Consume the PDB peripheral and return a driver.
    ///
    /// Enables the PDB clock gate (SIM_SCGC6.PDB).
    fn constrain(self, sim: &pac::Sim) -> Pdb;
}

impl PdbExt for pac::Pdb0 {
    fn constrain(self, sim: &pac::Sim) -> Pdb {
        // Enable PDB clock gate
        sim.scgc6().modify(|_, w| w.pdb()._1());
        Pdb { _private: () }
    }
}
