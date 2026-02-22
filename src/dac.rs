//! 12-bit DAC driver (DAC0, mk20d7 only).
//!
//! The MK20DX256 has a single 12-bit DAC with software trigger mode
//! and selectable voltage reference. The DAC output appears on pin
//! DAC0_OUT (PTE30/ALT0 on Teensy 3.1/3.2).
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... disable watchdog, configure clocks ...
//! let mut dac = dp.dac0.dac(&dp.sim);
//! dac.set_value(2048); // mid-scale (~1.65V with 3.3V VDDA)
//! ```

use crate::pac;

/// DAC voltage reference source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VrefSource {
    /// DACREF_1 (VDDA on Teensy — typically 3.3V).
    Vref1,
    /// DACREF_2 (VREF_OUT).
    Vref2,
}

/// 12-bit DAC driver for DAC0.
pub struct Dac {
    _private: (),
}

/// Extension trait to initialize DAC0 from the PAC peripheral.
pub trait DacExt {
    /// Enable clock gate, configure unbuffered software-trigger mode,
    /// and enable the DAC output. Output starts at 0.
    fn dac(self, sim: &pac::Sim) -> Dac;
}

impl DacExt for pac::Dac0 {
    fn dac(self, sim: &pac::Sim) -> Dac {
        // Enable DAC0 clock gate (SIM SCGC2)
        sim.scgc2().modify(|_, w| w.dac0()._1());

        let dac = unsafe { &*pac::Dac0::PTR };

        // Set output to 0
        dac.datl(0).write(|w| unsafe { w.data().bits(0) });
        dac.dath(0).write(|w| unsafe { w.data().bits(0) });

        // C1: buffer disabled, no DMA, VREF2 not used as top
        dac.c1().write(|w| w);

        // C2: buffer watermark = 0, upper limit = 0 (buffer disabled anyway)
        dac.c2().write(|w| w);

        // C0: enable DAC, VREF1 (VDDA), software trigger, high power,
        // no buffer interrupts
        dac.c0().write(|w| {
            w.dacen()._1()       // enable
             .dacrfs()._0()      // DACREF_1 (VDDA)
             .dactrgsel()._1()   // software trigger
             .lpen()._0()        // high power
        });

        Dac { _private: () }
    }
}

impl Dac {
    fn regs() -> &'static <pac::Dac0 as core::ops::Deref>::Target {
        unsafe { &*pac::Dac0::PTR }
    }

    /// Set the 12-bit DAC output value (0-4095).
    ///
    /// Values above 4095 are silently masked to 12 bits.
    pub fn set_value(&mut self, value: u16) {
        let dac = Self::regs();
        let value = value & 0x0FFF;
        dac.datl(0).write(|w| unsafe { w.data().bits(value as u8) });
        dac.dath(0).write(|w| unsafe { w.data().bits((value >> 8) as u8) });
    }

    /// Read the current 12-bit DAC output value.
    pub fn get_value(&self) -> u16 {
        let dac = Self::regs();
        let low = dac.datl(0).read().data().bits() as u16;
        let high = (dac.dath(0).read().data().bits() as u16) & 0x0F;
        (high << 8) | low
    }

    /// Select the voltage reference source.
    pub fn set_vref(&mut self, vref: VrefSource) {
        let dac = Self::regs();
        dac.c0().modify(|_, w| match vref {
            VrefSource::Vref1 => w.dacrfs()._0(),
            VrefSource::Vref2 => w.dacrfs()._1(),
        });
    }

    /// Enable the DAC output.
    pub fn enable(&mut self) {
        let dac = Self::regs();
        dac.c0().modify(|_, w| w.dacen()._1());
    }

    /// Disable the DAC output.
    pub fn disable(&mut self) {
        let dac = Self::regs();
        dac.c0().modify(|_, w| w.dacen()._0());
    }
}
