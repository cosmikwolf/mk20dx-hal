use crate::pac;

/// Extension trait to disable the watchdog timer.
///
/// The unlock sequence must write two magic values to WDOG_UNLOCK
/// within 20 bus clock cycles. After unlocking, the WDOGEN bit
/// must be cleared within 256 bus clock cycles.
///
/// # Call this within 256 bus clock cycles of reset, or the chip resets
///
/// K20 reference manual §23.3.2: "You must unlock the registers within WCT
/// [256 bus clock cycles] after system reset, failing which the WDOG issues a
/// reset to the system." `cortex_m_rt` copies `.data` and zeroes `.bss` before
/// `main`, which takes far longer than that on any firmware with real static
/// data. Calling `disable()` from `main` then comes too late: the chip
/// watchdog-resets during RAM init, about 20,000 times a second, and never
/// reaches `main` (measured on a MK20DX256 on 2026-09-10, `RCM_SRS0 = 0x20`).
///
/// **The failure is invisible under a debugger.** RM §23.5 suspends the rule
/// while the core is halted, and debug probes (probe-rs, J-Link, OpenOCD) also
/// disable the WDOG themselves. A firmware that "only runs with the debugger
/// attached" is the symptom.
///
/// A tiny binary with no `.data` and a word of `.bss` can reach `main` inside
/// the window by luck. Do not rely on that. Put the unlock in a
/// `#[cortex_m_rt::pre_init]` function, which runs before RAM init:
///
/// ```ignore
/// #[cortex_m_rt::pre_init]
/// unsafe fn disable_wdog_early() {
///     // SAFETY: nothing else runs yet; these are the first stores after reset.
///     let wdog = &*mk20dx_hal::pac::Wdog::ptr();
///     wdog.unlock().write(|w| w.bits(0xC520));
///     wdog.unlock().write(|w| w.bits(0xD928));
///     // RM 23.3.1 step 2: no update on the bus cycle right after the unlock.
///     cortex_m::asm::nop();
///     cortex_m::asm::nop();
///     wdog.stctrlh().write(|w| w.wdogen().disabled());
/// }
/// ```
///
/// Reset value of `STCTRLH` is `0x01D3`; the write above keeps `ALLOWUPDATE`
/// set, so a later `dp.wdog.disable()` in `main` stays legal (unlock, then one
/// update, as RM §23.3.1 allows after the window closes) and is harmless.
///
/// Build with at least `opt-level = 1`: at `opt-level = 0` the two unlock
/// stores are more than 20 bus cycles apart and the WDOG resets the chip on
/// the spot.
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
