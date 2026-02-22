//! Power mode control (SMC — System Mode Controller).
//!
//! The SMC manages power mode entry and transitions. Before entering
//! any low-power mode, the mode must first be allowed via [`PowerControl::allow_vlp`],
//! [`PowerControl::allow_lls`], or [`PowerControl::allow_vlls`].
//!
//! # Power Modes
//!
//! | Mode | Description | Wake Sources |
//! |------|-------------|--------------|
//! | Wait | CPU stopped, peripherals running | Any interrupt |
//! | VLPR | Reduced-frequency run mode | N/A (already running) |
//! | VLPW | Wait mode in VLPR | Any interrupt |
//! | VLPS | Very Low-Power Stop | Any interrupt |
//! | LLS  | Low-Leakage Stop | LLWU pins/modules, RESET |
//! | VLLSx | Very Low-Leakage Stop | LLWU pins, RESET |
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::power::{PowerControl, StopMode};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! let mut power = dp.smc.power_control(&dp.sim);
//! power.allow_vlp();
//!
//! // Enter VLPS — wakes on any enabled interrupt
//! power.enter_stop(StopMode::Vlps);
//! ```

use crate::pac;

/// Current power mode reported by PMSTAT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerMode {
    /// Normal Run mode.
    Run,
    /// Very Low-Power Run mode.
    Vlpr,
    /// Stop mode (one of STOP/VLPS/LLS/VLLSx — cannot distinguish after wake).
    Stop,
    /// Unknown status value.
    Unknown(u8),
}

/// Stop mode selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum StopMode {
    /// Normal STOP (wake on any interrupt).
    NormalStop,
    /// Very Low-Power Stop (wake on any interrupt).
    Vlps,
    /// Low-Leakage Stop (wake via LLWU only).
    Lls,
    /// Very Low-Leakage Stop 3 (SRAM retained, wake via LLWU).
    Vlls3,
    /// Very Low-Leakage Stop 2 (partial SRAM, wake via LLWU).
    Vlls2,
    /// Very Low-Leakage Stop 1 (minimal retention, wake via LLWU).
    Vlls1,
}

/// SMC power control driver.
pub struct PowerControl {
    _private: (),
}

/// Extension trait to initialize SMC from the PAC peripheral.
pub trait SmcExt {
    /// Enable the SMC and return a power control driver.
    fn power_control(self, sim: &pac::Sim) -> PowerControl;
}

impl SmcExt for pac::Smc {
    fn power_control(self, _sim: &pac::Sim) -> PowerControl {
        // SMC has no clock gate — it's always accessible.
        PowerControl { _private: () }
    }
}

impl PowerControl {
    fn regs() -> &'static pac::smc::RegisterBlock {
        unsafe { &*pac::Smc::PTR }
    }

    /// Allow Very Low-Power modes (VLPR, VLPW, VLPS).
    ///
    /// Must be called before entering any VLP mode. This can only be
    /// written once after reset — subsequent writes are ignored.
    pub fn allow_vlp(&mut self) {
        Self::regs().pmprot().modify(|_, w| w.avlp()._1());
    }

    /// Allow Low-Leakage Stop (LLS) mode.
    ///
    /// Must be called before entering LLS. This can only be written
    /// once after reset — subsequent writes are ignored.
    pub fn allow_lls(&mut self) {
        Self::regs().pmprot().modify(|_, w| w.alls()._1());
    }

    /// Allow Very Low-Leakage Stop (VLLSx) modes.
    ///
    /// Must be called before entering any VLLS mode. This can only be
    /// written once after reset — subsequent writes are ignored.
    pub fn allow_vlls(&mut self) {
        Self::regs().pmprot().modify(|_, w| w.avlls()._1());
    }

    /// Allow all low-power modes at once.
    ///
    /// Convenience method that enables VLP, LLS, and VLLS.
    /// Can only be written once after reset.
    pub fn allow_all(&mut self) {
        Self::regs().pmprot().write(|w| {
            w.avlp()._1()
             .alls()._1()
             .avlls()._1()
        });
    }

    /// Enter Normal Wait mode (WFI).
    ///
    /// CPU is stopped, but all peripherals continue running.
    /// Wakes on any enabled interrupt. This does not require
    /// any PMPROT permissions.
    pub fn enter_wait(&self) {
        let smc = Self::regs();

        // Ensure normal run mode and normal stop
        smc.pmctrl().modify(|_, w| w.stopm()._000());

        // DSB + WFI
        cortex_m::asm::dsb();
        cortex_m::asm::wfi();
    }

    /// Enter a stop mode.
    ///
    /// The appropriate mode must first be allowed via `allow_vlp()`,
    /// `allow_lls()`, or `allow_vlls()`. If not allowed, the stop
    /// entry will be aborted by hardware.
    ///
    /// For LLS and VLLS modes, configure LLWU wake sources before
    /// calling this method.
    ///
    /// Returns `true` if stop was entered successfully, `false` if
    /// the stop entry was aborted (STOPA flag set).
    pub fn enter_stop(&mut self, mode: StopMode) -> bool {
        let smc = Self::regs();

        match mode {
            StopMode::NormalStop => {
                smc.pmctrl().modify(|_, w| w.stopm()._000());
            }
            StopMode::Vlps => {
                smc.pmctrl().modify(|_, w| w.stopm()._010());
            }
            StopMode::Lls => {
                smc.pmctrl().modify(|_, w| w.stopm()._011());
            }
            StopMode::Vlls3 => {
                smc.pmctrl().modify(|_, w| w.stopm()._100());
                smc.vllsctrl().write(|w| w.vllsm()._011());
            }
            StopMode::Vlls2 => {
                smc.pmctrl().modify(|_, w| w.stopm()._100());
                smc.vllsctrl().write(|w| w.vllsm()._010());
            }
            StopMode::Vlls1 => {
                smc.pmctrl().modify(|_, w| w.stopm()._100());
                smc.vllsctrl().write(|w| w.vllsm()._001());
            }
        }

        // Set SLEEPDEEP bit in SCB for stop modes
        // Safety: we only set SCR.SLEEPDEEP, which is required for stop entry
        unsafe {
            let scb = &*cortex_m::peripheral::SCB::PTR;
            scb.scr.modify(|v| v | (1 << 2)); // SLEEPDEEP
        }

        // DSB to ensure all memory accesses complete, then WFI
        cortex_m::asm::dsb();
        cortex_m::asm::wfi();
        cortex_m::asm::isb();

        // Clear SLEEPDEEP after wake
        unsafe {
            let scb = &*cortex_m::peripheral::SCB::PTR;
            scb.scr.modify(|v| v & !(1 << 2));
        }

        // Check if stop was aborted
        !smc.pmctrl().read().stopa().is_1()
    }

    /// Enter Very Low-Power Run (VLPR) mode.
    ///
    /// The MCG must first be switched to BLPI or BLPE mode, and
    /// the SIM clock dividers must be set for reduced frequencies
    /// (core <= 4 MHz for MK20D7, <= 2 MHz for MK20D5).
    ///
    /// Call [`allow_vlp`](PowerControl::allow_vlp) before this method.
    ///
    /// Returns `true` if VLPR entry was acknowledged.
    pub fn enter_vlpr(&mut self) -> bool {
        let smc = Self::regs();

        smc.pmctrl().modify(|_, w| w.runm()._10());

        // Wait for VLPR to be confirmed
        // PMSTAT should read 0x04 for VLPR
        let mut timeout = 1000u32;
        while smc.pmstat().read().pmstat().bits() != 4 {
            timeout -= 1;
            if timeout == 0 {
                return false;
            }
        }

        true
    }

    /// Exit Very Low-Power Run (VLPR) mode back to normal Run.
    ///
    /// After exiting, the MCG should be transitioned back to PEE
    /// mode and clock dividers restored.
    pub fn exit_vlpr(&mut self) {
        let smc = Self::regs();

        smc.pmctrl().modify(|_, w| w.runm()._00());

        // Wait for normal run mode (PMSTAT = 0x01)
        while smc.pmstat().read().pmstat().bits() != 1 {}
    }

    /// Read the current power mode.
    pub fn current_mode(&self) -> PowerMode {
        match Self::regs().pmstat().read().pmstat().bits() {
            1 => PowerMode::Run,
            4 => PowerMode::Vlpr,
            // After waking from stop, PMSTAT returns to the run mode value
            other => PowerMode::Unknown(other),
        }
    }

    /// Check if the last stop mode entry was aborted.
    pub fn stop_aborted(&self) -> bool {
        Self::regs().pmctrl().read().stopa().is_1()
    }

    /// Release the SMC peripheral, returning the PAC type.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers.
    pub unsafe fn release(self) -> pac::Smc {
        pac::Smc::steal()
    }
}
