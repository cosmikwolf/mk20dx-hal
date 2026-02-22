use cortex_m::peripheral::syst::SystClkSource;
use cortex_m::peripheral::SYST;
use embedded_hal::delay::DelayNs;

use crate::clocks::Clocks;

/// SysTick-based delay provider.
///
/// Implements [`DelayNs`] using the ARM Cortex-M SysTick timer.
/// The SysTick reload register is 24 bits wide, limiting single
/// delays to ~233 ms at 72 MHz. Longer delays are handled by
/// looping.
pub struct Delay {
    syst: SYST,
    cycles_per_us: u32,
}

impl Delay {
    /// Create a new `Delay` from the SysTick peripheral and frozen clocks.
    pub fn new(mut syst: SYST, clocks: &Clocks) -> Self {
        syst.set_clock_source(SystClkSource::Core);
        let cycles_per_us = clocks.core_clk().raw() / 1_000_000;
        Delay { syst, cycles_per_us }
    }

    /// Release the SysTick peripheral.
    pub fn free(self) -> SYST {
        self.syst
    }
}

impl DelayNs for Delay {
    fn delay_ns(&mut self, ns: u32) {
        // Convert ns to cycles, rounding up
        let cycles = (ns as u64 * self.cycles_per_us as u64 + 999) / 1000;

        // SysTick reload is 24 bits (max 0x00FF_FFFF = 16_777_215)
        const MAX_RELOAD: u64 = 0x00FF_FFFF;
        let mut remaining = cycles;

        while remaining > 0 {
            let reload = if remaining > MAX_RELOAD {
                MAX_RELOAD as u32
            } else {
                remaining as u32
            };

            if reload > 0 {
                self.syst.set_reload(reload);
                self.syst.clear_current();
                self.syst.enable_counter();

                while !self.syst.has_wrapped() {}

                self.syst.disable_counter();
            }

            remaining = remaining.saturating_sub(reload as u64);
        }
    }
}
