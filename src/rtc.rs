//! Real-Time Clock (RTC) driver.
//!
//! The RTC uses an independent 32.768 kHz crystal oscillator (present on
//! all Teensy 3.x boards) and counts seconds in a 32-bit register. When
//! powered by VBAT, the RTC maintains time across resets.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... disable watchdog, configure clocks ...
//! let mut rtc = dp.rtc.rtc(&dp.sim);
//!
//! if !rtc.time_is_valid() {
//!     rtc.set_time(1700000000); // set to a known epoch
//! }
//! let now = rtc.seconds().unwrap();
//! ```

use crate::pac;

/// Error returned when reading time while the Time Invalid Flag is set.
///
/// This occurs after initial power-on (before time has been set) or
/// after a software reset of the RTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TimeInvalid;

/// Real-Time Clock driver.
pub struct Rtc {
    _private: (),
}

/// Extension trait to initialize the RTC from the PAC peripheral.
pub trait RtcExt {
    /// Enable the RTC clock gate and 32.768 kHz oscillator.
    ///
    /// Preserves existing time if VBAT was maintained. Enables the
    /// oscillator with ~10 pF load capacitance (SC8P + SC2P) if not
    /// already running. Disables all interrupts. Starts the counter
    /// if time is valid.
    ///
    /// Does NOT block for the ~500 ms oscillator startup time.
    fn rtc(self, sim: &pac::Sim) -> Rtc;
}

impl RtcExt for pac::Rtc {
    fn rtc(self, sim: &pac::Sim) -> Rtc {
        // Enable RTC clock gate (SIM SCGC6)
        sim.scgc6().modify(|_, w| w.rtc().enabled());

        let rtc = unsafe { &*pac::Rtc::PTR };

        // Clear software reset if set
        if rtc.cr().read().swr().is_1() {
            rtc.cr().modify(|_, w| w.swr()._0());
        }

        // Enable 32.768 kHz oscillator with ~10 pF load caps if not already running
        if rtc.cr().read().osce().is_0() {
            rtc.cr().modify(|_, w| {
                w.osce()._1()   // enable oscillator
                 .sc8p()._1()   // 8 pF
                 .sc2p()._1()   // 2 pF (total ~10 pF)
            });
        }

        // Disable all interrupts
        rtc.ier().write(|w| {
            w.tiie()._0()
             .toie()._0()
             .taie()._0()
             .tsie()._0()
        });

        // Start counter if time is valid (TIF == 0)
        if rtc.sr().read().tif().is_0() && rtc.sr().read().tce().is_0() {
            rtc.sr().write(|w| w.tce()._1());
        }

        Rtc { _private: () }
    }
}

impl Rtc {
    fn regs() -> &'static <pac::Rtc as core::ops::Deref>::Target {
        unsafe { &*pac::Rtc::PTR }
    }

    /// Returns `true` if the time is valid (TIF is not set).
    ///
    /// Time becomes invalid after initial power-on or software reset
    /// until `set_time()` is called.
    pub fn time_is_valid(&self) -> bool {
        Self::regs().sr().read().tif().is_0()
    }

    /// Read the current seconds counter.
    ///
    /// Returns `Err(TimeInvalid)` if the Time Invalid Flag is set,
    /// meaning the time has not been initialized.
    pub fn seconds(&self) -> Result<u32, TimeInvalid> {
        let rtc = Self::regs();
        if rtc.sr().read().tif().is_1() {
            return Err(TimeInvalid);
        }
        Ok(rtc.tsr().read().tsr().bits())
    }

    /// Set the seconds counter to the given value.
    ///
    /// The time counter must be disabled to write TSR, so this method
    /// temporarily disables the counter, writes the value, then
    /// re-enables it.
    pub fn set_time(&mut self, seconds: u32) {
        let rtc = Self::regs();

        // Disable counter (TCE=0) — required before writing TSR
        rtc.sr().write(|w| w.tce()._0());

        // Write time seconds register
        rtc.tsr().write(|w| unsafe { w.tsr().bits(seconds) });

        // Re-enable counter
        rtc.sr().write(|w| w.tce()._1());
    }

    /// Set the alarm time (in seconds).
    ///
    /// When the seconds counter matches this value, the Time Alarm Flag
    /// (TAF) is set. Enable the alarm interrupt with
    /// `enable_alarm_interrupt()` to receive an interrupt.
    pub fn set_alarm(&mut self, seconds: u32) {
        let rtc = Self::regs();
        rtc.tar().write(|w| unsafe { w.tar().bits(seconds) });
    }

    /// Returns `true` if the alarm has fired (TAF is set).
    pub fn alarm_fired(&self) -> bool {
        Self::regs().sr().read().taf().is_1()
    }

    /// Clear the alarm by writing 0 to TAR (clears TAF).
    pub fn clear_alarm(&mut self) {
        let rtc = Self::regs();
        rtc.tar().write(|w| unsafe { w.tar().bits(0) });
    }

    /// Enable the time alarm interrupt (TAIE).
    pub fn enable_alarm_interrupt(&mut self) {
        Self::regs().ier().modify(|_, w| w.taie()._1());
    }

    /// Disable the time alarm interrupt (TAIE).
    pub fn disable_alarm_interrupt(&mut self) {
        Self::regs().ier().modify(|_, w| w.taie()._0());
    }

    /// Enable the seconds interrupt (TSIE).
    ///
    /// Fires once per second when the counter increments.
    pub fn enable_seconds_interrupt(&mut self) {
        Self::regs().ier().modify(|_, w| w.tsie()._1());
    }

    /// Disable the seconds interrupt (TSIE).
    pub fn disable_seconds_interrupt(&mut self) {
        Self::regs().ier().modify(|_, w| w.tsie()._0());
    }

    /// Enable the time counter (TCE=1).
    pub fn enable(&mut self) {
        Self::regs().sr().write(|w| w.tce()._1());
    }

    /// Disable the time counter (TCE=0).
    pub fn disable(&mut self) {
        Self::regs().sr().write(|w| w.tce()._0());
    }

    /// Release the RTC peripheral, returning the PAC type.
    ///
    /// The RTC counter continues running — this does not disable the clock
    /// or oscillator, preserving timekeeping.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers.
    pub unsafe fn release(self) -> pac::Rtc {
        pac::Rtc::steal()
    }
}
