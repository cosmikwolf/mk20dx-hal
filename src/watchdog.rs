use crate::pac;

/// Extension trait to disable the watchdog timer.
///
/// The unlock sequence must write two magic values to WDOG_UNLOCK
/// within 20 bus clock cycles. After unlocking, the WDOGEN bit
/// must be cleared within 256 bus clock cycles.
pub trait WdogExt {
    /// Disable the watchdog timer, consuming the WDOG peripheral.
    fn disable(self);
}

impl WdogExt for pac::Wdog {
    fn disable(self) {
        // Unlock sequence: write 0xC520 then 0xD928 in quick succession
        cortex_m::interrupt::free(|_| {
            // SAFETY: The unlock sequence writes two magic values (0xC520, 0xD928)
            // to WDOG_UNLOCK. The bits() call is safe because the UNLOCK register
            // is a 16-bit write-only register that accepts any value. The critical
            // section ensures the two writes complete within the 20-bus-cycle window.
            unsafe {
                self.unlock().write(|w| w.bits(0xC520));
                self.unlock().write(|w| w.bits(0xD928));
            }

            // Disable the watchdog — must happen within 256 bus clocks of unlock
            self.stctrlh().write(|w| w.wdogen().disabled());
        });
    }
}
