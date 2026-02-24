//! PIT (Periodic Interrupt Timer) driver.
//!
//! The PIT provides 4 independent 32-bit down-counter channels clocked from
//! the bus clock. Both MK20D5 and MK20D7 have identical PIT hardware.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::time::U32Ext;
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... configure clocks ...
//! let pit = dp.PIT.split(&dp.SIM, &clocks);
//! let mut ch0 = pit.ch0;
//! ch0.start(1_000_000u32.micros()); // 1 second period
//! nb::block!(ch0.wait()).ok();
//! ```

use crate::clocks::Clocks;
use crate::pac;

/// Access the PIT register block.
fn regs() -> &'static pac::pit::RegisterBlock {
    // SAFETY: PTR is a valid pointer to the PIT register block.
    unsafe { &*pac::Pit::PTR }
}

/// PIT channels returned by [`PitExt::split`].
pub struct PitChannels {
    pub ch0: PitChannel<0>,
    pub ch1: PitChannel<1>,
    pub ch2: PitChannel<2>,
    pub ch3: PitChannel<3>,
}

/// A single PIT timer channel.
///
/// Each channel is an independent 32-bit down-counter. The period is
/// `(LDVAL + 1) / bus_clk` seconds.
pub struct PitChannel<const CH: u8> {
    bus_clk: u32,
}

impl<const CH: u8> PitChannel<CH> {
    /// Start the timer with a period in microseconds.
    ///
    /// Restarts the counter from the new load value immediately.
    /// Maximum period depends on bus clock (~119 s at 36 MHz, ~179 s at 24 MHz).
    ///
    /// **Note:** Values exceeding the 32-bit counter range are silently
    /// clamped to `u32::MAX` ticks. Use [`start_ticks()`](PitChannel::start_ticks)
    /// for precise control.
    pub fn start(&mut self, period: fugit::MicrosDurationU32) {
        let us = period.ticks() as u64;
        let ticks = (us * self.bus_clk as u64 / 1_000_000)
            .saturating_sub(1)
            .min(u32::MAX as u64) as u32;
        self.start_ticks(ticks);
    }

    /// Start the timer with a raw tick count (LDVAL = ticks).
    ///
    /// Period = (ticks + 1) / bus_clk seconds.
    /// Disables the timer, loads the new value, clears any pending flag,
    /// then re-enables the timer.
    pub fn start_ticks(&mut self, ticks: u32) {
        let pit = regs();
        let ch = CH as usize;
        // Disable → load → clear flag → enable
        pit.tctrl(ch).modify(|_, w| w.ten()._0());
        // SAFETY: tsv is a 32-bit field that accepts any u32 value.
        pit.ldval(ch).write(|w| unsafe { w.tsv().bits(ticks) });
        pit.tflg(ch).write(|w| w.tif()._1());
        pit.tctrl(ch).modify(|_, w| w.ten()._1());
    }

    /// Poll for timer expiry.
    ///
    /// Returns `Ok(())` when the timer has expired (TIF set), clearing the flag.
    /// Returns `Err(WouldBlock)` if the timer hasn't expired yet.
    pub fn wait(&mut self) -> nb::Result<(), core::convert::Infallible> {
        let pit = regs();
        let ch = CH as usize;
        if pit.tflg(ch).read().tif().is_1() {
            pit.tflg(ch).write(|w| w.tif()._1());
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }

    /// Stop the timer.
    pub fn cancel(&mut self) {
        regs().tctrl(CH as usize).modify(|_, w| w.ten()._0());
    }

    /// Read the current down-counter value.
    pub fn current(&self) -> u32 {
        regs().cval(CH as usize).read().tvl().bits()
    }

    /// Enable the timer interrupt.
    ///
    /// Clears any pending interrupt flag first to avoid an immediate interrupt.
    pub fn enable_interrupt(&mut self) {
        let pit = regs();
        let ch = CH as usize;
        pit.tflg(ch).write(|w| w.tif()._1());
        pit.tctrl(ch).modify(|_, w| w.tie()._1());
    }

    /// Disable the timer interrupt.
    pub fn disable_interrupt(&mut self) {
        regs().tctrl(CH as usize).modify(|_, w| w.tie()._0());
    }

    /// Check if the timer interrupt flag is set (without clearing it).
    pub fn has_expired(&self) -> bool {
        regs().tflg(CH as usize).read().tif().is_1()
    }

    /// Clear the timer interrupt flag.
    pub fn clear_interrupt(&mut self) {
        regs().tflg(CH as usize).write(|w| w.tif()._1());
    }
}

/// Extension trait for the PIT peripheral.
///
/// Provides `split()` to consume the PAC PIT and return individual timer channels.
pub trait PitExt {
    /// Consume the PIT peripheral and return 4 independent timer channels.
    ///
    /// Enables the PIT clock gate, enables the module, and configures freeze
    /// in debug mode.
    fn split(self, sim: &pac::Sim, clocks: &Clocks) -> PitChannels;
}

impl PitExt for pac::Pit {
    fn split(self, sim: &pac::Sim, clocks: &Clocks) -> PitChannels {
        // Enable PIT clock gate
        sim.scgc6().modify(|_, w| w.pit()._1());

        // Enable PIT module (MDIS=0) and freeze in debug mode (FRZ=1)
        let pit = regs();
        pit.mcr().write(|w| w.mdis()._0().frz()._1());

        let bus_clk = clocks.bus_clk().raw();
        PitChannels {
            ch0: PitChannel { bus_clk },
            ch1: PitChannel { bus_clk },
            ch2: PitChannel { bus_clk },
            ch3: PitChannel { bus_clk },
        }
    }
}

// ----- Async support -----

#[cfg(feature = "async")]
mod async_impl {
    //! Async support assumes a single-executor, single-core environment.
    //! Each PIT channel has one waker — only one task may await a given
    //! channel at a time.

    use super::*;
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use embassy_sync::waitqueue::AtomicWaker;

    static PIT_WAKERS: [AtomicWaker; 4] = [
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
    ];

    /// Call from the `PIT0` interrupt handler.
    pub fn on_pit0_interrupt() {
        on_pit_interrupt(0);
    }

    /// Call from the `PIT1` interrupt handler.
    pub fn on_pit1_interrupt() {
        on_pit_interrupt(1);
    }

    /// Call from the `PIT2` interrupt handler.
    pub fn on_pit2_interrupt() {
        on_pit_interrupt(2);
    }

    /// Call from the `PIT3` interrupt handler.
    pub fn on_pit3_interrupt() {
        on_pit_interrupt(3);
    }

    fn on_pit_interrupt(ch: usize) {
        let pit = regs();
        if pit.tflg(ch).read().tif().is_1() {
            // Clear TIF flag (w1c)
            pit.tflg(ch).write(|w| w.tif()._1());
            // Disable interrupt to prevent repeated firing
            pit.tctrl(ch).modify(|_, w| w.tie()._0());
            PIT_WAKERS[ch].wake();
        }
    }

    /// Future that resolves when a PIT channel's interrupt fires.
    struct PitFuture<const CH: u8>;

    impl<const CH: u8> Future for PitFuture<CH> {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let ch = CH as usize;
            PIT_WAKERS[ch].register(cx.waker());
            let pit = regs();
            // Check if already fired (TIE disabled by ISR means it completed)
            if pit.tctrl(ch).read().tie().is_0() && pit.tctrl(ch).read().ten().is_1() {
                // Timer is running but interrupt is disabled — ISR already fired
                Poll::Ready(())
            } else if pit.tflg(ch).read().tif().is_1() {
                // Flag set but ISR hasn't run yet (race) — clear it ourselves
                pit.tflg(ch).write(|w| w.tif()._1());
                pit.tctrl(ch).modify(|_, w| w.tie()._0());
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    impl<const CH: u8> PitChannel<CH> {
        /// Async one-shot delay in microseconds.
        ///
        /// Configures the PIT channel, enables its interrupt, and awaits
        /// completion. The caller must wire the corresponding `PIT{N}`
        /// interrupt to [`on_pitN_interrupt()`] and unmask it in the NVIC.
        pub async fn delay_us(&mut self, us: u32) {
            if us == 0 {
                return;
            }
            let ticks = (us as u64 * self.bus_clk as u64 / 1_000_000)
                .saturating_sub(1)
                .min(u32::MAX as u64) as u32;
            let pit = regs();
            let ch = CH as usize;
            // Stop → load → clear flag → enable with interrupt
            pit.tctrl(ch).modify(|_, w| w.ten()._0());
            // SAFETY: tsv is a 32-bit field that accepts any u32 value.
            pit.ldval(ch).write(|w| unsafe { w.tsv().bits(ticks) });
            pit.tflg(ch).write(|w| w.tif()._1());
            pit.tctrl(ch).write(|w| w.ten()._1().tie()._1());
            PitFuture::<CH>.await;
        }
    }

    impl<const CH: u8> embedded_hal_async::delay::DelayNs for PitChannel<CH> {
        async fn delay_ns(&mut self, ns: u32) {
            let us = ns.saturating_add(999) / 1000;
            if us > 0 {
                self.delay_us(us).await;
            }
        }

        async fn delay_us(&mut self, us: u32) {
            PitChannel::delay_us(self, us).await;
        }

        async fn delay_ms(&mut self, ms: u32) {
            PitChannel::delay_us(self, ms.saturating_mul(1000)).await;
        }
    }
}

#[cfg(feature = "async")]
pub use async_impl::{on_pit0_interrupt, on_pit1_interrupt, on_pit2_interrupt, on_pit3_interrupt};
