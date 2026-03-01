//! Low-Power Timer (LPTMR0) driver.
//!
//! The LPTMR is a 16-bit timer that can operate in time counter or pulse
//! counter mode. It can run from the LPO (1 kHz), ERCLK32K (32.768 kHz),
//! or other clock sources, and continues to run in low-power modes.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::lptmr::{LptmrClock, Lptmr};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... disable watchdog, configure clocks ...
//! let mut lptmr = dp.lptmr0.lptmr(&dp.sim);
//! lptmr.start(1000, LptmrClock::Lpo1kHz); // 1 second period
//! loop {
//!     if lptmr.wait().is_ok() {
//!         // Timer fired
//!     }
//! }
//! ```

use core::convert::Infallible;

use crate::pac;

/// LPTMR clock source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LptmrClock {
    /// MCGIRCLK (internal reference clock).
    McgIrClk,
    /// LPO (1 kHz low-power oscillator).
    Lpo1kHz,
    /// ERCLK32K (32.768 kHz RTC oscillator).
    ErClk32k,
    /// OSCERCLK (external reference clock).
    OscErClk,
}

/// LPTMR prescaler divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Prescaler {
    /// Bypass prescaler (divide by 1).
    Bypass,
    /// Divide by 2.
    Div2,
    /// Divide by 4.
    Div4,
    /// Divide by 8.
    Div8,
    /// Divide by 16.
    Div16,
    /// Divide by 32.
    Div32,
    /// Divide by 64.
    Div64,
    /// Divide by 128.
    Div128,
    /// Divide by 256.
    Div256,
    /// Divide by 512.
    Div512,
    /// Divide by 1024.
    Div1024,
    /// Divide by 2048.
    Div2048,
    /// Divide by 4096.
    Div4096,
    /// Divide by 8192.
    Div8192,
    /// Divide by 16384.
    Div16384,
    /// Divide by 32768.
    Div32768,
    /// Divide by 65536.
    Div65536,
}

/// Low-Power Timer driver.
pub struct Lptmr {
    _private: (),
}

/// Extension trait to initialize LPTMR0 from the PAC peripheral.
pub trait LptmrExt {
    /// Enable the LPTMR clock gate and return a driver handle.
    fn lptmr(self, sim: &pac::Sim) -> Lptmr;
}

impl LptmrExt for pac::Lptmr0 {
    fn lptmr(self, sim: &pac::Sim) -> Lptmr {
        // Enable LPTMR clock gate (SIM SCGC5)
        sim.scgc5().modify(|_, w| w.lptimer().enabled());

        let lptmr = Lptmr::regs();

        // Disable timer during configuration
        lptmr.csr().write(|w| w.ten()._0());

        Lptmr { _private: () }
    }
}

impl Lptmr {
    fn regs() -> &'static pac::lptmr0::RegisterBlock {
        unsafe { &*pac::Lptmr0::PTR }
    }

    /// Start the timer in time counter mode with a period in milliseconds.
    ///
    /// Uses the specified clock source with the prescaler bypassed.
    /// The compare value is calculated assuming:
    /// - `Lpo1kHz`: 1 tick per ms
    /// - `ErClk32k`: 32.768 ticks per ms
    /// - Others: compare set to `period_ms` directly (user must account for clock rate)
    ///
    /// The timer resets on compare match (TFC=0) and the compare flag is set.
    pub fn start(&mut self, period_ms: u32, clock: LptmrClock) {
        let lptmr = Self::regs();

        // Disable timer
        lptmr.csr().write(|w| w.ten()._0());

        // Set clock source, bypass prescaler
        lptmr.psr().write(|w| {
            let w = w.pbyp()._1(); // bypass prescaler
            match clock {
                LptmrClock::McgIrClk => w.pcs()._00(),
                LptmrClock::Lpo1kHz => w.pcs()._01(),
                LptmrClock::ErClk32k => w.pcs()._10(),
                LptmrClock::OscErClk => w.pcs()._11(),
            }
        });

        // Calculate compare value
        let compare = match clock {
            LptmrClock::Lpo1kHz => period_ms.min(65535) as u16,
            LptmrClock::ErClk32k => {
                let ticks = (period_ms as u64 * 32768 / 1000).min(65535);
                ticks as u16
            }
            _ => period_ms.min(65535) as u16,
        };

        // Set compare value (must be > 0)
        let compare = compare.max(1);
        lptmr.cmr().write(|w| unsafe { w.compare().bits(compare) });

        // Enable timer: time counter mode, reset on compare, no interrupt yet
        lptmr.csr().write(|w| {
            w.ten()._1()
             .tms()._0()  // time counter mode
             .tfc()._0()  // reset on compare match
        });
    }

    /// Start the timer with a specific prescaler and compare value.
    ///
    /// For precise control over the timer period.
    pub fn start_raw(&mut self, clock: LptmrClock, prescaler: Prescaler, compare: u16) {
        let lptmr = Self::regs();

        // Disable timer
        lptmr.csr().write(|w| w.ten()._0());

        // Set clock source and prescaler
        lptmr.psr().write(|w| {
            let w = match clock {
                LptmrClock::McgIrClk => w.pcs()._00(),
                LptmrClock::Lpo1kHz => w.pcs()._01(),
                LptmrClock::ErClk32k => w.pcs()._10(),
                LptmrClock::OscErClk => w.pcs()._11(),
            };
            match prescaler {
                Prescaler::Bypass => w.pbyp()._1(),
                p => {
                    let val = match p {
                        Prescaler::Div2 => 0,
                        Prescaler::Div4 => 1,
                        Prescaler::Div8 => 2,
                        Prescaler::Div16 => 3,
                        Prescaler::Div32 => 4,
                        Prescaler::Div64 => 5,
                        Prescaler::Div128 => 6,
                        Prescaler::Div256 => 7,
                        Prescaler::Div512 => 8,
                        Prescaler::Div1024 => 9,
                        Prescaler::Div2048 => 10,
                        Prescaler::Div4096 => 11,
                        Prescaler::Div8192 => 12,
                        Prescaler::Div16384 => 13,
                        Prescaler::Div32768 => 14,
                        Prescaler::Div65536 => 15,
                        Prescaler::Bypass => unreachable!(),
                    };
                    w.pbyp()._0();
                    unsafe { w.prescale().bits(val) }
                }
            }
        });

        let compare = compare.max(1);
        lptmr.cmr().write(|w| unsafe { w.compare().bits(compare) });

        // Enable timer
        lptmr.csr().write(|w| {
            w.ten()._1()
             .tms()._0()
             .tfc()._0()
        });
    }

    /// Non-blocking poll for compare match.
    ///
    /// Returns `Ok(())` when the compare flag is set, or `WouldBlock` if
    /// the timer hasn't fired yet. Clears the flag on success.
    pub fn wait(&mut self) -> nb::Result<(), Infallible> {
        let lptmr = Self::regs();
        if lptmr.csr().read().tcf().is_1() {
            // Clear TCF by writing 1 to it (w1c)
            lptmr.csr().modify(|_, w| w.tcf()._1());
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }

    /// Cancel the timer (disable it).
    pub fn cancel(&mut self) {
        Self::regs().csr().write(|w| w.ten()._0());
    }

    /// Read the current counter value.
    ///
    /// Note: On Kinetis, reading CNR requires writing any value to CNR first
    /// to latch the current count. This is handled automatically.
    pub fn count(&self) -> u16 {
        let lptmr = Self::regs();
        // Write to CNR to latch the current counter value
        lptmr.cnr().write(|w| unsafe { w.counter().bits(0) });
        lptmr.cnr().read().counter().bits()
    }

    /// Enable the timer interrupt (TIE).
    pub fn enable_interrupt(&mut self) {
        Self::regs().csr().modify(|_, w| w.tie()._1());
    }

    /// Disable the timer interrupt (TIE).
    pub fn disable_interrupt(&mut self) {
        Self::regs().csr().modify(|_, w| w.tie()._0());
    }

    /// Clear the compare flag (TCF).
    pub fn clear_flag(&mut self) {
        Self::regs().csr().modify(|_, w| w.tcf()._1());
    }

    /// Release the LPTMR peripheral, returning the PAC type.
    ///
    /// Disables the timer before releasing.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers.
    pub unsafe fn release(self) -> pac::Lptmr0 {
        Self::regs().csr().write(|w| w.ten()._0());
        pac::Lptmr0::steal()
    }
}
