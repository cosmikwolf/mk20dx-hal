use core::convert::Infallible;
use core::marker::PhantomData;

use crate::clocks::Clocks;
use crate::pac;
use crate::time::Hertz;

// ----- Instance Abstraction -----

mod sealed {
    pub trait FtmInstance {
        fn ptr() -> *const crate::pac::ftm0::RegisterBlock;
        fn enable_clock(sim: &crate::pac::Sim);
    }
}

fn ftm_regs<FTM: sealed::FtmInstance>() -> &'static pac::ftm0::RegisterBlock {
    // SAFETY: FtmInstance::ptr() returns a valid register block pointer.
    unsafe { &*FTM::ptr() }
}

// ----- Instance Markers -----

/// Marker type for FTM0.
pub struct Ftm0;

/// Marker type for FTM1.
pub struct Ftm1;

/// Marker type for FTM2 (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Ftm2;

impl sealed::FtmInstance for Ftm0 {
    fn ptr() -> *const pac::ftm0::RegisterBlock {
        pac::Ftm0::PTR
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc6().modify(|_, w| w.ftm0()._1());
    }
}

impl sealed::FtmInstance for Ftm1 {
    fn ptr() -> *const pac::ftm0::RegisterBlock {
        // SAFETY: FTM1 has identical register layout to FTM0.
        pac::Ftm1::PTR as *const pac::ftm0::RegisterBlock
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc6().modify(|_, w| w.ftm1()._1());
    }
}

#[cfg(feature = "mk20d7")]
impl sealed::FtmInstance for Ftm2 {
    fn ptr() -> *const pac::ftm0::RegisterBlock {
        // SAFETY: FTM2 has identical register layout to FTM0.
        pac::Ftm2::PTR as *const pac::ftm0::RegisterBlock
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc3().modify(|_, w| w.ftm2()._1());
    }
}

// ----- Channel Type -----

/// A single PWM output channel of an FTM peripheral.
///
/// Implements [`embedded_hal::pwm::SetDutyCycle`]. Call [`enable`](PwmChannel::enable)
/// to start PWM output and [`disable`](PwmChannel::disable) to stop it.
///
/// Pin MUX must be configured separately to route the FTM channel to a
/// physical pin (typically ALT3 or ALT4 depending on the pin).
pub struct PwmChannel<FTM, const CH: u8> {
    mod_val: u16,
    _ftm: PhantomData<FTM>,
}

// ----- Channel Sets -----

/// PWM channels for FTM0 (8 channels).
pub struct Ftm0Channels {
    pub ch0: PwmChannel<Ftm0, 0>,
    pub ch1: PwmChannel<Ftm0, 1>,
    pub ch2: PwmChannel<Ftm0, 2>,
    pub ch3: PwmChannel<Ftm0, 3>,
    pub ch4: PwmChannel<Ftm0, 4>,
    pub ch5: PwmChannel<Ftm0, 5>,
    pub ch6: PwmChannel<Ftm0, 6>,
    pub ch7: PwmChannel<Ftm0, 7>,
}

/// PWM channels for FTM1 (2 channels).
pub struct Ftm1Channels {
    pub ch0: PwmChannel<Ftm1, 0>,
    pub ch1: PwmChannel<Ftm1, 1>,
}

/// PWM channels for FTM2 (2 channels, mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Ftm2Channels {
    pub ch0: PwmChannel<Ftm2, 0>,
    pub ch1: PwmChannel<Ftm2, 1>,
}

// ----- Prescaler Calculation -----

/// Calculate prescaler index and modulo value for a target PWM frequency.
///
/// Returns `(ps_idx, mod_val)` where `ps_idx` is 0..7 mapping to div1..div128,
/// and `mod_val` is the MOD register value (period = MOD + 1 counts).
/// Prescaler divisors map to FTM SC PS field values 0-7
/// (K20 ref manual Table 36-30 / K20P64M72SF1RM).
pub fn calc_prescaler(bus_clk: u32, target_freq: u32) -> (u8, u16) {
    const DIVS: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
    for (idx, &div) in DIVS.iter().enumerate() {
        let counter_clk = bus_clk / div;
        let mod_val = counter_clk / target_freq;
        if mod_val > 0 && mod_val <= 65536 {
            return (idx as u8, (mod_val - 1) as u16);
        }
    }
    // Fallback: max prescaler, max period
    (7, 0xFFFF)
}

// ----- Extension Trait -----

/// Extension trait for creating PWM channels from PAC FTM peripherals.
///
/// Consumes the FTM peripheral and configures it for edge-aligned PWM
/// at the requested frequency. Pin MUX must be configured separately
/// (typically ALT3 or ALT4 depending on the pin).
pub trait FtmExt: Sized {
    type Channels;

    /// Configure edge-aligned PWM at the given frequency.
    ///
    /// Consumes the FTM peripheral. All channels start disabled;
    /// call [`PwmChannel::enable`] on individual channels to start output.
    fn pwm(self, frequency: Hertz, clocks: &Clocks, sim: &pac::Sim) -> Self::Channels;
}

// ----- Per-instance macro -----

macro_rules! ftm_pwm_impl {
    ($PacType:ty, $Instance:ty, $Channels:ident {
        $($ch_name:ident : $ch_idx:literal),+
    }, $scgc_reg:ident, $scgc_field:ident) => {
        impl<const CH: u8> PwmChannel<$Instance, CH> {
            fn regs() -> &'static <$PacType as core::ops::Deref>::Target {
                // SAFETY: PTR is a valid pointer to the FTM register block.
                unsafe { &*<$PacType>::PTR }
            }

            /// Enable PWM output on this channel (edge-aligned, high-true).
            ///
            /// Sets MSB:MSA=10, ELSB:ELSA=10 for edge-aligned PWM with
            /// high-true pulses (output set on counter wrap, cleared on CnV match).
            pub fn enable(&mut self) {
                let ftm = Self::regs();
                ftm.csc(CH as usize).write(|w| {
                    w.msb().set_bit()
                     .elsb().set_bit()
                });
            }

            /// Disable PWM output on this channel.
            pub fn disable(&mut self) {
                let ftm = Self::regs();
                ftm.csc(CH as usize).write(|w| w);
            }

            /// Enable DMA requests on channel match.
            ///
            /// When enabled, the FTM channel match event triggers a DMA request
            /// (via DMAMUX source FTM0_CHn) instead of a CPU interrupt.
            /// Both CnSC.DMA and CnSC.CHIE must be set for DMA triggering.
            pub fn enable_dma(&mut self) {
                let ftm = Self::regs();
                ftm.csc(CH as usize).modify(|_, w| w.dma()._1().chie()._1());
            }

            /// Disable DMA requests on channel match.
            pub fn disable_dma(&mut self) {
                let ftm = Self::regs();
                ftm.csc(CH as usize).modify(|_, w| w.dma()._0().chie()._0());
            }
        }

        impl<const CH: u8> embedded_hal::pwm::ErrorType for PwmChannel<$Instance, CH> {
            type Error = core::convert::Infallible;
        }

        impl<const CH: u8> embedded_hal::pwm::SetDutyCycle for PwmChannel<$Instance, CH> {
            fn max_duty_cycle(&self) -> u16 {
                self.mod_val
            }

            fn set_duty_cycle(&mut self, duty: u16) -> Result<(), Self::Error> {
                let ftm = Self::regs();
                // SAFETY: val is a 16-bit field; duty is u16.
                ftm.cv(CH as usize).write(|w| unsafe { w.val().bits(duty) });
                Ok(())
            }
        }

        impl FtmExt for $PacType {
            type Channels = $Channels;

            fn pwm(self, frequency: Hertz, clocks: &Clocks, sim: &pac::Sim) -> $Channels {
                let bus_clk = clocks.bus_clk().raw();
                let (ps_idx, mod_val) = calc_prescaler(bus_clk, frequency.raw());

                // Enable clock gate
                sim.$scgc_reg().modify(|_, w| w.$scgc_field()._1());

                // SAFETY: PTR is a valid pointer to the FTM register block.
                let ftm = unsafe { &*<$PacType>::PTR };

                // 1. Disable counter (CLKS=None)
                ftm.sc().write(|w| w.clks().none());

                // 2. Disable write protection
                ftm.mode().write(|w| w.wpdis()._1());

                // SAFETY: init/mod_/count are 16-bit fields; values fit.
                ftm.cntin().write(|w| unsafe { w.init().bits(0) });

                // 4. Set modulo (period)
                ftm.mod_().write(|w| unsafe { w.mod_().bits(mod_val) });

                // 5. Write to CNT to sync counter with CNTIN
                ftm.cnt().write(|w| unsafe { w.count().bits(0) });

                // 6. Enable counter: system clock, edge-aligned (CPWMS=0), prescaler
                ftm.sc().write(|w| {
                    let w = w.clks().system().cpwms()._0();
                    match ps_idx {
                        0 => w.ps().div1(),
                        1 => w.ps().div2(),
                        2 => w.ps().div4(),
                        3 => w.ps().div8(),
                        4 => w.ps().div16(),
                        5 => w.ps().div32(),
                        6 => w.ps().div64(),
                        _ => w.ps().div128(),
                    }
                });

                // All channels start disabled (CnSC = 0 from reset)
                $Channels {
                    $($ch_name: PwmChannel { mod_val, _ftm: PhantomData },)+
                }
            }
        }
    };
}

// Both variants have FTM0 (8 channels) and FTM1 (2 channels)
ftm_pwm_impl!(pac::Ftm0, Ftm0, Ftm0Channels {
    ch0:0, ch1:1, ch2:2, ch3:3, ch4:4, ch5:5, ch6:6, ch7:7
}, scgc6, ftm0);

ftm_pwm_impl!(pac::Ftm1, Ftm1, Ftm1Channels {
    ch0:0, ch1:1
}, scgc6, ftm1);

// Only mk20d7 has FTM2 (2 channels)
#[cfg(feature = "mk20d7")]
ftm_pwm_impl!(pac::Ftm2, Ftm2, Ftm2Channels {
    ch0:0, ch1:1
}, scgc3, ftm2);

// =====================================================================
// Input Capture
// =====================================================================

/// Edge detection mode for input capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CaptureEdge {
    /// Capture on rising edge only.
    Rising,
    /// Capture on falling edge only.
    Falling,
    /// Capture on both rising and falling edges.
    Both,
}

/// Input capture channel.
///
/// Captures the FTM counter value when the configured edge is detected
/// on the channel input. Pin MUX must be configured separately.
pub struct InputCapture<FTM, const CH: u8> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: sealed::FtmInstance, const CH: u8> InputCapture<FTM, CH> {
    /// Configure a channel for input capture mode.
    ///
    /// The FTM must already be initialized (clock enabled, counter running).
    /// Typically call this after `FtmExt::pwm()` or after manually configuring
    /// the FTM counter via direct register access.
    ///
    /// # Arguments
    /// * `edge` — Which edges trigger a capture.
    /// * `clocks` — Clocks token (for clock gating).
    /// * `sim` — SIM peripheral (for clock gating).
    pub fn new(edge: CaptureEdge, clocks: &Clocks, sim: &pac::Sim) -> Self {
        let _ = clocks;
        FTM::enable_clock(sim);
        let ftm = ftm_regs::<FTM>();

        // Disable write protection
        ftm.mode().modify(|_, w| w.wpdis()._1());

        // Configure channel for input capture: MSB:MSA = 00
        ftm.csc(CH as usize).write(|w| {
            match edge {
                CaptureEdge::Rising => w.elsa().set_bit(),
                CaptureEdge::Falling => w.elsb().set_bit(),
                CaptureEdge::Both => w.elsa().set_bit().elsb().set_bit(),
            }
        });

        // Ensure counter is running (system clock, if not already set)
        if ftm.sc().read().clks().is_none() {
            ftm.sc().modify(|_, w| w.clks().system());
        }

        InputCapture { _ftm: PhantomData }
    }

    /// Read the last captured value if the channel flag is set.
    ///
    /// Returns `Some(value)` and clears the flag, or `None` if no capture
    /// has occurred since the last read.
    pub fn capture(&self) -> Option<u16> {
        let ftm = ftm_regs::<FTM>();
        if ftm.csc(CH as usize).read().chf().is_1() {
            let val = ftm.cv(CH as usize).read().val().bits();
            // Clear CHF by reading CnSC then writing 0 to CHF
            // (CHF is cleared by reading CnSC when CHF is set, then writing 0)
            ftm.csc(CH as usize).modify(|_, w| w);
            Some(val)
        } else {
            None
        }
    }

    /// Non-blocking poll for a capture event.
    pub fn wait(&mut self) -> nb::Result<u16, Infallible> {
        match self.capture() {
            Some(val) => Ok(val),
            None => Err(nb::Error::WouldBlock),
        }
    }

    /// Enable the channel interrupt (CHIE).
    pub fn enable_interrupt(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.chie().set_bit());
    }

    /// Disable the channel interrupt (CHIE).
    pub fn disable_interrupt(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.chie().clear_bit());
    }

    /// Set the input filter value for this channel (0-15).
    ///
    /// Only channels 0-3 have hardware filters. Values for channels 4-7
    /// are ignored.
    pub fn set_filter(&mut self, value: u8) {
        let ftm = ftm_regs::<FTM>();
        let value = value & 0x0F;
        match CH {
            // SAFETY: chNfval are 4-bit fields; value is masked to 0x0F above.
            0 => { ftm.filter().modify(|_, w| unsafe { w.ch0fval().bits(value) }); },
            1 => { ftm.filter().modify(|_, w| unsafe { w.ch1fval().bits(value) }); },
            2 => { ftm.filter().modify(|_, w| unsafe { w.ch2fval().bits(value) }); },
            3 => { ftm.filter().modify(|_, w| unsafe { w.ch3fval().bits(value) }); },
            _ => {} // Channels 4-7 have no filter
        }
    }

    /// Clear the channel flag (CHF).
    pub fn clear_flag(&self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w);
    }

    /// Enable DMA requests on channel capture.
    ///
    /// Both CnSC.DMA and CnSC.CHIE must be set for DMA triggering.
    pub fn enable_dma(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.dma()._1().chie()._1());
    }

    /// Disable DMA requests on channel capture.
    pub fn disable_dma(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.dma()._0().chie()._0());
    }
}

// =====================================================================
// Output Compare
// =====================================================================

/// Action to perform when the counter matches the compare value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CompareAction {
    /// Toggle the channel output on match.
    Toggle,
    /// Clear the channel output on match (drive low).
    Clear,
    /// Set the channel output on match (drive high).
    Set,
}

/// Output compare channel.
///
/// Performs an action on the channel output when the FTM counter matches
/// the compare value. Pin MUX must be configured separately.
pub struct OutputCompare<FTM, const CH: u8> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: sealed::FtmInstance, const CH: u8> OutputCompare<FTM, CH> {
    /// Configure a channel for output compare mode.
    ///
    /// The FTM must already be initialized (clock enabled, counter running).
    ///
    /// # Arguments
    /// * `action` — What to do when counter matches CnV.
    /// * `compare` — Initial compare value.
    /// * `clocks` — Clocks token.
    /// * `sim` — SIM peripheral.
    pub fn new(
        action: CompareAction,
        compare: u16,
        clocks: &Clocks,
        sim: &pac::Sim,
    ) -> Self {
        let _ = clocks;
        FTM::enable_clock(sim);
        let ftm = ftm_regs::<FTM>();

        // Disable write protection
        ftm.mode().modify(|_, w| w.wpdis()._1());

        // SAFETY: val is a 16-bit field; compare is u16.
        ftm.cv(CH as usize).write(|w| unsafe { w.val().bits(compare) });

        // Configure channel for output compare: MSB=0, MSA=1
        ftm.csc(CH as usize).write(|w| {
            let w = w.msa().set_bit();
            match action {
                CompareAction::Toggle => w.elsa().set_bit(),
                CompareAction::Clear => w.elsb().set_bit(),
                CompareAction::Set => w.elsa().set_bit().elsb().set_bit(),
            }
        });

        // Ensure counter is running
        if ftm.sc().read().clks().is_none() {
            ftm.sc().modify(|_, w| w.clks().system());
        }

        OutputCompare { _ftm: PhantomData }
    }

    /// Set the compare value (CnV).
    pub fn set_compare(&mut self, value: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: val is a 16-bit field; value is u16.
        ftm.cv(CH as usize).write(|w| unsafe { w.val().bits(value) });
    }

    /// Change the compare action.
    pub fn set_action(&mut self, action: CompareAction) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|r, w| {
            // Preserve CHIE bit
            let w = if r.chie().is_1() { w.chie().set_bit() } else { w };
            let w = w.msa().set_bit();
            match action {
                CompareAction::Toggle => w.elsa().set_bit(),
                CompareAction::Clear => w.elsb().set_bit(),
                CompareAction::Set => w.elsa().set_bit().elsb().set_bit(),
            }
        });
    }

    /// Enable the channel interrupt (CHIE).
    pub fn enable_interrupt(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.chie().set_bit());
    }

    /// Disable the channel interrupt (CHIE).
    pub fn disable_interrupt(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.chie().clear_bit());
    }

    /// Check if the channel flag is set (match occurred).
    pub fn has_matched(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).read().chf().is_1()
    }

    /// Clear the channel flag (CHF).
    pub fn clear_flag(&self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w);
    }

    /// Enable DMA requests on channel match.
    ///
    /// Both CnSC.DMA and CnSC.CHIE must be set for DMA triggering.
    pub fn enable_dma(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.dma()._1().chie()._1());
    }

    /// Disable DMA requests on channel match.
    pub fn disable_dma(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.dma()._0().chie()._0());
    }
}

// =====================================================================
// Quadrature Decoder
// =====================================================================

/// Quadrature decoder encoding mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum QuadMode {
    /// Phase A and Phase B encoding (standard quadrature).
    PhaseAB,
    /// Count and direction encoding (Phase A = clock, Phase B = direction).
    CountDirection,
}

/// Counting direction reported by the quadrature decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Direction {
    /// Counter is incrementing.
    Up,
    /// Counter is decrementing.
    Down,
}

/// Quadrature decoder using FTM Phase A (CH0) and Phase B (CH1) inputs.
///
/// All FTM instances support quadrature mode. Phase A uses the channel 0
/// pin, Phase B uses the channel 1 pin. Pin MUX must be configured
/// separately.
pub struct QuadratureDecoder<FTM> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: sealed::FtmInstance> QuadratureDecoder<FTM> {
    /// Configure the FTM for quadrature decoder mode.
    ///
    /// Enables the FTM clock gate, sets FTMEN and QUADEN, and configures
    /// the encoding mode. The counter counts based on Phase A/B input edges.
    ///
    /// # Arguments
    /// * `mode` — Quadrature encoding mode.
    /// * `sim` — SIM peripheral for clock gating.
    pub fn new(mode: QuadMode, sim: &pac::Sim) -> Self {
        FTM::enable_clock(sim);
        let ftm = ftm_regs::<FTM>();

        // Disable counter
        ftm.sc().write(|w| w.clks().none());

        // Enable FTM features mode and disable write protection
        ftm.mode().write(|w| w.ftmen()._1().wpdis()._1());

        // SAFETY: init/mod_/count are 16-bit fields; values fit.
        ftm.cntin().write(|w| unsafe { w.init().bits(0) });
        ftm.mod_().write(|w| unsafe { w.mod_().bits(0xFFFF) });
        ftm.cnt().write(|w| unsafe { w.count().bits(0) });

        // Configure quadrature decoder
        ftm.qdctrl().write(|w| {
            let w = w.quaden()._1();
            match mode {
                QuadMode::PhaseAB => w.quadmode()._0(),
                QuadMode::CountDirection => w.quadmode()._1(),
            }
        });

        QuadratureDecoder { _ftm: PhantomData }
    }

    /// Read the current counter value.
    pub fn count(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.cnt().read().count().bits()
    }

    /// Read the counting direction.
    pub fn direction(&self) -> Direction {
        let ftm = ftm_regs::<FTM>();
        if ftm.qdctrl().read().quadir().is_1() {
            Direction::Up
        } else {
            Direction::Down
        }
    }

    /// Reset the counter to zero.
    pub fn reset_count(&mut self) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: count is a 16-bit field; 0 fits.
        ftm.cnt().write(|w| unsafe { w.count().bits(0) });
    }

    /// Set the modulo value (counter wraps at MOD).
    pub fn set_modulo(&mut self, modulo: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: mod_ is a 16-bit field; modulo is u16.
        ftm.mod_().write(|w| unsafe { w.mod_().bits(modulo) });
    }

    /// Enable the timer overflow interrupt (TOIE).
    pub fn enable_overflow_interrupt(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.sc().modify(|_, w| w.toie()._1());
    }

    /// Disable the timer overflow interrupt (TOIE).
    pub fn disable_overflow_interrupt(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.sc().modify(|_, w| w.toie()._0());
    }

    /// Check and clear the timer overflow flag (TOF).
    ///
    /// Returns `true` if overflow occurred. The flag is cleared by the
    /// read-modify-write cycle (reading SC when TOF=1, then writing 0
    /// to the TOF bit position).
    pub fn overflow_flag(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        let sc = ftm.sc().read();
        let tof = sc.tof().is_1();
        if tof {
            // Clear TOF: modify() reads SC (TOF=1 seen), then writes back
            // with TOF bit = 0 (since we don't set it), which clears it.
            ftm.sc().modify(|_, w| w);
        }
        tof
    }

    /// Set input filter values for Phase A (CH0) and Phase B (CH1).
    ///
    /// Filter value 0 disables the filter. Values 1-15 set the filter
    /// period to `value` bus clock cycles.
    pub fn set_filter(&mut self, phase_a: u8, phase_b: u8) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: chNfval are 4-bit fields; values masked to 0x0F.
        ftm.filter().modify(|_, w| unsafe {
            w.ch0fval().bits(phase_a & 0x0F)
             .ch1fval().bits(phase_b & 0x0F)
        });
        ftm.qdctrl().modify(|_, w| {
            let w = if phase_a > 0 { w.phafltren()._1() } else { w.phafltren()._0() };
            if phase_b > 0 { w.phbfltren()._1() } else { w.phbfltren()._0() }
        });
    }

    /// Set input polarity for Phase A and Phase B.
    ///
    /// When `true`, the input is inverted (active-low).
    pub fn set_polarity(&mut self, phase_a_invert: bool, phase_b_invert: bool) {
        let ftm = ftm_regs::<FTM>();
        ftm.qdctrl().modify(|_, w| {
            let w = if phase_a_invert { w.phapol()._1() } else { w.phapol()._0() };
            if phase_b_invert { w.phbpol()._1() } else { w.phbpol()._0() }
        });
    }
}
