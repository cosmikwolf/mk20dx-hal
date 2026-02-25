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

// =====================================================================
// Prescaler
// =====================================================================

/// FTM counter clock prescaler divisor.
///
/// Maps directly to the SC.PS field values 0-7
/// (K20 ref manual Table 36-30 / K20P64M72SF1RM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Prescaler {
    Div1,
    Div2,
    Div4,
    Div8,
    Div16,
    Div32,
    Div64,
    Div128,
}

impl Prescaler {
    /// Convert prescaler to the SC.PS field index (0-7).
    fn to_idx(self) -> u8 {
        match self {
            Prescaler::Div1 => 0,
            Prescaler::Div2 => 1,
            Prescaler::Div4 => 2,
            Prescaler::Div8 => 3,
            Prescaler::Div16 => 4,
            Prescaler::Div32 => 5,
            Prescaler::Div64 => 6,
            Prescaler::Div128 => 7,
        }
    }

    /// Convert SC.PS field index to a Prescaler variant.
    fn from_idx(idx: u8) -> Self {
        match idx {
            0 => Prescaler::Div1,
            1 => Prescaler::Div2,
            2 => Prescaler::Div4,
            3 => Prescaler::Div8,
            4 => Prescaler::Div16,
            5 => Prescaler::Div32,
            6 => Prescaler::Div64,
            _ => Prescaler::Div128,
        }
    }
}

// =====================================================================
// PWM Alignment
// =====================================================================

/// PWM alignment mode (timer-wide setting).
///
/// Controls whether the FTM counter counts up only (edge-aligned) or
/// up-down (center-aligned). Center-aligned mode produces symmetric
/// waveforms, useful for motor control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PwmAlignment {
    /// Edge-aligned PWM (counter counts up only). Period = MOD+1 ticks.
    EdgeAligned,
    /// Center-aligned PWM (counter counts up-down). Period = 2*MOD ticks.
    CenterAligned,
}

// =====================================================================
// FtmTimer — shared counter/period handle
// =====================================================================

/// Central FTM timer handle for shared counter, period, and prescaler operations.
///
/// Returned by [`FtmExt::split`]. Wraps the shared FTM registers (SC, MOD, CNT)
/// in safe methods. All methods use static register pointers internally (zero-cost).
pub struct FtmTimer<FTM> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: sealed::FtmInstance> FtmTimer<FTM> {
    /// Stop the counter (CLKS=None).
    ///
    /// With the counter stopped, MOD and CnV writes take effect immediately
    /// (bypassing the write buffer that exists when the counter is running).
    pub fn stop(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.sc().modify(|_, w| w.clks().none());
    }

    /// Start the counter with the system (bus) clock.
    pub fn start(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.sc().modify(|_, w| w.clks().system());
    }

    /// Whether the counter is currently running (CLKS != None).
    pub fn is_running(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        !ftm.sc().read().clks().is_none()
    }

    /// Set the modulo (period) value.
    ///
    /// The counter counts from CNTIN (0) to MOD, then wraps. The PWM period
    /// is MOD + 1 counter ticks.
    ///
    /// If the counter is running, the write goes to a buffer and takes effect
    /// at the next counter overflow. If the counter is stopped, the write
    /// takes effect immediately.
    pub fn set_modulo(&mut self, mod_val: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: mod_ is a 16-bit field; mod_val is u16.
        ftm.mod_().write(|w| unsafe { w.mod_().bits(mod_val) });
    }

    /// Read the current modulo value.
    pub fn modulo(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.mod_().read().mod_().bits()
    }

    /// Reset the counter to CNTIN (0).
    ///
    /// Writing any value to CNT loads CNTIN into the counter.
    pub fn reset_counter(&mut self) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: count is a 16-bit field; 0 fits.
        ftm.cnt().write(|w| unsafe { w.count().bits(0) });
    }

    /// Read the current counter value.
    pub fn counter(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.cnt().read().count().bits()
    }

    /// Set the prescaler divisor.
    ///
    /// The prescaler divides the bus clock before it reaches the FTM counter.
    /// Changing the prescaler while the counter is running takes effect
    /// after the current prescaler count completes.
    pub fn set_prescaler(&mut self, ps: Prescaler) {
        let ftm = ftm_regs::<FTM>();
        let idx = ps.to_idx();
        ftm.sc().modify(|_, w| {
            match idx {
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
    }

    /// Read the current prescaler setting.
    pub fn prescaler(&self) -> Prescaler {
        let ftm = ftm_regs::<FTM>();
        Prescaler::from_idx(ftm.sc().read().ps().bits())
    }

    /// Calculate and set prescaler + MOD from a target frequency.
    ///
    /// Uses the bus clock from `clocks` to find the best prescaler and
    /// modulo combination. Stops the counter, writes MOD, sets the
    /// prescaler, and resets the counter. Does **not** restart — call
    /// [`start`](Self::start) afterward.
    pub fn set_frequency(&mut self, freq: Hertz, clocks: &Clocks) {
        let bus_clk = clocks.bus_clk().raw();
        let (ps_idx, mod_val) = calc_prescaler(bus_clk, freq.raw());

        let ftm = ftm_regs::<FTM>();

        // Stop counter so MOD write is immediate
        ftm.sc().modify(|_, w| w.clks().none());

        // Write MOD
        // SAFETY: mod_ is a 16-bit field; mod_val is u16.
        ftm.mod_().write(|w| unsafe { w.mod_().bits(mod_val) });

        // Reset counter
        // SAFETY: count is a 16-bit field; 0 fits.
        ftm.cnt().write(|w| unsafe { w.count().bits(0) });

        // Preserve current CPWMS setting
        let cpwms = ftm.sc().read().cpwms().bit();

        // Set prescaler (counter stays stopped)
        ftm.sc().write(|w| {
            let w = w.clks().none();
            let w = if cpwms { w.cpwms()._1() } else { w.cpwms()._0() };
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
    }

    /// Set PWM alignment mode (edge-aligned or center-aligned).
    ///
    /// CPWMS is a timer-wide setting that affects all channels.
    /// Should be set while counter is stopped (CLKS=None); otherwise
    /// the change takes effect at the next counter overflow.
    pub fn set_alignment(&mut self, alignment: PwmAlignment) {
        let ftm = ftm_regs::<FTM>();
        ftm.sc().modify(|_, w| match alignment {
            PwmAlignment::EdgeAligned => w.cpwms()._0(),
            PwmAlignment::CenterAligned => w.cpwms()._1(),
        });
    }

    /// Read the current alignment mode.
    pub fn alignment(&self) -> PwmAlignment {
        let ftm = ftm_regs::<FTM>();
        if ftm.sc().read().cpwms().bit() {
            PwmAlignment::CenterAligned
        } else {
            PwmAlignment::EdgeAligned
        }
    }
}

// =====================================================================
// PWM Polarity
// =====================================================================

/// PWM output polarity.
///
/// Controls whether the active portion of the PWM cycle drives the
/// output high or low. Works with both edge-aligned and center-aligned modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PwmPolarity {
    /// High-true pulses (output high during active period). ELSB=1, ELSA=0.
    HighTrue,
    /// Low-true pulses (output low during active period). ELSB=0, ELSA=1.
    LowTrue,
}

// =====================================================================
// FtmChannel — unified per-channel handle
// =====================================================================

/// A single FTM channel handle supporting PWM, output compare, input capture, and DMA.
///
/// Returned by [`FtmExt::split`] as part of the FTM parts struct. Each channel
/// can be independently configured for different modes. Implements
/// [`embedded_hal::pwm::SetDutyCycle`] for PWM use.
///
/// Pin MUX must be configured separately to route the FTM channel to a
/// physical pin (typically ALT3 or ALT4 depending on the pin).
pub struct FtmChannel<FTM, const CH: u8> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: sealed::FtmInstance, const CH: u8> FtmChannel<FTM, CH> {
    // --- Mode configuration ---

    /// Configure for edge-aligned PWM (high-true pulses).
    ///
    /// Sets MSB:MSA=10, ELSB:ELSA=10. Output is set on counter wrap
    /// (MOD→CNTIN) and cleared on CnV match.
    pub fn set_pwm(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).write(|w| {
            w.msb().set_bit()
             .elsb().set_bit()
        });
    }

    /// Configure for PWM with explicit polarity.
    ///
    /// `HighTrue`: output is high during the active duty cycle (default).
    /// `LowTrue`: output is low during the active duty cycle (inverted).
    ///
    /// Works with both edge-aligned and center-aligned modes.
    pub fn set_pwm_polarity(&mut self, polarity: PwmPolarity) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).write(|w| {
            let w = w.msb().set_bit();
            match polarity {
                PwmPolarity::HighTrue => w.elsb().set_bit(),
                PwmPolarity::LowTrue => w.elsa().set_bit(),
            }
        });
    }

    /// Configure for output compare mode.
    ///
    /// Sets MSB=0, MSA=1, ELS per action. The channel output performs
    /// `action` when the counter matches `compare`.
    pub fn set_output_compare(&mut self, action: CompareAction, compare: u16) {
        let ftm = ftm_regs::<FTM>();
        // Configure CnSC for output compare BEFORE writing CnV.
        ftm.csc(CH as usize).write(|w| {
            let w = w.msa().set_bit();
            match action {
                CompareAction::Toggle => w.elsa().set_bit(),
                CompareAction::Clear => w.elsb().set_bit(),
                CompareAction::Set => w.elsa().set_bit().elsb().set_bit(),
            }
        });
        // SAFETY: val is a 16-bit field; compare is u16.
        ftm.cv(CH as usize).write(|w| unsafe { w.val().bits(compare) });
    }

    /// Configure for input capture mode.
    ///
    /// Sets MSB:MSA=00, ELS per edge. The channel captures the counter
    /// value when the configured edge is detected.
    pub fn set_input_capture(&mut self, edge: CaptureEdge) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).write(|w| {
            match edge {
                CaptureEdge::Rising => w.elsa().set_bit(),
                CaptureEdge::Falling => w.elsb().set_bit(),
                CaptureEdge::Both => w.elsa().set_bit().elsb().set_bit(),
            }
        });
    }

    /// Disable the channel (CnSC = 0).
    pub fn disable(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).write(|w| w);
    }

    // --- Value access ---

    /// Set the compare/duty value (CnV).
    ///
    /// For PWM: sets duty cycle. For output compare: sets match value.
    /// If the counter is running, the write goes to a buffer and latches
    /// at the next counter overflow. If stopped, takes effect immediately.
    pub fn set_value(&mut self, val: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: val is a 16-bit field; val is u16.
        ftm.cv(CH as usize).write(|w| unsafe { w.val().bits(val) });
    }

    /// Read the current CnV value.
    pub fn value(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.cv(CH as usize).read().val().bits()
    }

    // --- Flag & interrupt ---

    /// Check if the channel flag (CHF) is set.
    ///
    /// For PWM/OC: set on CnV match. For IC: set on edge capture.
    pub fn has_flag(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).read().chf().is_1()
    }

    /// Clear the channel flag (CHF).
    ///
    /// CHF is cleared by reading CnSC (with CHF=1) then writing 0 to
    /// the CHF bit position; `modify()` accomplishes this.
    pub fn clear_flag(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w);
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

    // --- DMA ---

    /// Enable DMA requests on channel events.
    ///
    /// Both CnSC.DMA and CnSC.CHIE must be set for DMA triggering.
    /// This method sets both bits, preserving the channel mode configuration.
    pub fn enable_dma(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.dma()._1().chie()._1());
    }

    /// Disable DMA requests on channel events.
    pub fn disable_dma(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(CH as usize).modify(|_, w| w.dma()._0().chie()._0());
    }

    // --- Input capture convenience ---

    /// Read the captured value if the channel flag is set, clearing the flag.
    ///
    /// Returns `Some(value)` if a capture occurred, `None` otherwise.
    pub fn capture(&mut self) -> Option<u16> {
        let ftm = ftm_regs::<FTM>();
        if ftm.csc(CH as usize).read().chf().is_1() {
            let val = ftm.cv(CH as usize).read().val().bits();
            ftm.csc(CH as usize).modify(|_, w| w);
            Some(val)
        } else {
            None
        }
    }
}

impl<FTM: sealed::FtmInstance, const CH: u8> embedded_hal::pwm::ErrorType for FtmChannel<FTM, CH> {
    type Error = Infallible;
}

impl<FTM: sealed::FtmInstance, const CH: u8> embedded_hal::pwm::SetDutyCycle for FtmChannel<FTM, CH> {
    fn max_duty_cycle(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.mod_().read().mod_().bits()
    }

    fn set_duty_cycle(&mut self, duty: u16) -> Result<(), Self::Error> {
        self.set_value(duty);
        Ok(())
    }
}

// ----- Channel Sets (legacy, backward compat) -----

/// PWM channels for FTM0 (8 channels).
///
/// Returned by [`FtmExt::pwm`] for backward compatibility.
/// New code should use [`Ftm0Parts`] via [`FtmExt::split`].
pub struct Ftm0Channels {
    pub ch0: FtmChannel<Ftm0, 0>,
    pub ch1: FtmChannel<Ftm0, 1>,
    pub ch2: FtmChannel<Ftm0, 2>,
    pub ch3: FtmChannel<Ftm0, 3>,
    pub ch4: FtmChannel<Ftm0, 4>,
    pub ch5: FtmChannel<Ftm0, 5>,
    pub ch6: FtmChannel<Ftm0, 6>,
    pub ch7: FtmChannel<Ftm0, 7>,
}

/// PWM channels for FTM1 (2 channels).
///
/// Returned by [`FtmExt::pwm`] for backward compatibility.
/// New code should use [`Ftm1Parts`] via [`FtmExt::split`].
pub struct Ftm1Channels {
    pub ch0: FtmChannel<Ftm1, 0>,
    pub ch1: FtmChannel<Ftm1, 1>,
}

/// PWM channels for FTM2 (2 channels, mk20d7 only).
///
/// Returned by [`FtmExt::pwm`] for backward compatibility.
/// New code should use [`Ftm2Parts`] via [`FtmExt::split`].
#[cfg(feature = "mk20d7")]
pub struct Ftm2Channels {
    pub ch0: FtmChannel<Ftm2, 0>,
    pub ch1: FtmChannel<Ftm2, 1>,
}

// ----- Parts structs (new split API) -----

/// Result of splitting FTM0 — timer handle + 8 channel handles.
pub struct Ftm0Parts {
    pub timer: FtmTimer<Ftm0>,
    pub ch0: FtmChannel<Ftm0, 0>,
    pub ch1: FtmChannel<Ftm0, 1>,
    pub ch2: FtmChannel<Ftm0, 2>,
    pub ch3: FtmChannel<Ftm0, 3>,
    pub ch4: FtmChannel<Ftm0, 4>,
    pub ch5: FtmChannel<Ftm0, 5>,
    pub ch6: FtmChannel<Ftm0, 6>,
    pub ch7: FtmChannel<Ftm0, 7>,
}

/// Result of splitting FTM1 — timer handle + 2 channel handles.
pub struct Ftm1Parts {
    pub timer: FtmTimer<Ftm1>,
    pub ch0: FtmChannel<Ftm1, 0>,
    pub ch1: FtmChannel<Ftm1, 1>,
}

/// Result of splitting FTM2 — timer handle + 2 channel handles (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Ftm2Parts {
    pub timer: FtmTimer<Ftm2>,
    pub ch0: FtmChannel<Ftm2, 0>,
    pub ch1: FtmChannel<Ftm2, 1>,
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

/// Extension trait for FTM peripherals.
///
/// Provides two entry points:
/// - [`split`](FtmExt::split) — returns a timer handle + per-channel handles.
///   Use this for multi-mode FTM usage (PWM + OC + IC on the same timer).
/// - [`pwm`](FtmExt::pwm) — convenience wrapper that configures for PWM at a
///   given frequency and returns only channel handles. Use this for simple
///   single-frequency PWM.
///
/// Both consume the PAC FTM peripheral to prevent register aliasing.
/// Pin MUX must be configured separately (typically ALT3 or ALT4).
pub trait FtmExt: Sized {
    type Channels;
    type Parts;

    /// Split the FTM into a timer handle and per-channel handles.
    ///
    /// Enables the clock gate, disables write protection, sets CNTIN=0,
    /// and returns parts with the counter stopped. Call
    /// [`FtmTimer::set_frequency`] or [`FtmTimer::set_modulo`] +
    /// [`FtmTimer::start`] to begin operation.
    fn split(self, clocks: &Clocks, sim: &pac::Sim) -> Self::Parts;

    /// Configure edge-aligned PWM at the given frequency.
    ///
    /// Convenience wrapper: calls `split()`, sets frequency, starts the
    /// counter, and returns only channel handles (timer handle is discarded).
    /// All channels start disabled; call [`FtmChannel::set_pwm`] on
    /// individual channels to start output.
    fn pwm(self, frequency: Hertz, clocks: &Clocks, sim: &pac::Sim) -> Self::Channels;
}

// ----- Per-instance macro -----

macro_rules! ftm_impl {
    ($PacType:ty, $Instance:ty,
     $Parts:ident { $($ch_name:ident : $ch_idx:literal),+ },
     $Channels:ident { $($ch_name2:ident : $ch_idx2:literal),+ },
     $scgc_reg:ident, $scgc_field:ident
    ) => {
        impl FtmExt for $PacType {
            type Channels = $Channels;
            type Parts = $Parts;

            fn split(self, clocks: &Clocks, sim: &pac::Sim) -> $Parts {
                let _ = clocks;

                // Enable clock gate
                sim.$scgc_reg().modify(|_, w| w.$scgc_field()._1());

                let ftm = ftm_regs::<$Instance>();

                // Stop counter
                ftm.sc().write(|w| w.clks().none());

                // Disable write protection
                ftm.mode().write(|w| w.wpdis()._1());

                // SAFETY: init/count are 16-bit fields; 0 fits.
                ftm.cntin().write(|w| unsafe { w.init().bits(0) });
                ftm.cnt().write(|w| unsafe { w.count().bits(0) });

                $Parts {
                    timer: FtmTimer { _ftm: PhantomData },
                    $($ch_name: FtmChannel { _ftm: PhantomData },)+
                }
            }

            fn pwm(self, frequency: Hertz, clocks: &Clocks, sim: &pac::Sim) -> $Channels {
                let bus_clk = clocks.bus_clk().raw();
                let (ps_idx, mod_val) = calc_prescaler(bus_clk, frequency.raw());

                // Enable clock gate
                sim.$scgc_reg().modify(|_, w| w.$scgc_field()._1());

                let ftm = ftm_regs::<$Instance>();

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
                    $($ch_name2: FtmChannel { _ftm: PhantomData },)+
                }
            }
        }
    };
}

// Both variants have FTM0 (8 channels) and FTM1 (2 channels)
ftm_impl!(pac::Ftm0, Ftm0,
    Ftm0Parts { ch0:0, ch1:1, ch2:2, ch3:3, ch4:4, ch5:5, ch6:6, ch7:7 },
    Ftm0Channels { ch0:0, ch1:1, ch2:2, ch3:3, ch4:4, ch5:5, ch6:6, ch7:7 },
    scgc6, ftm0
);

ftm_impl!(pac::Ftm1, Ftm1,
    Ftm1Parts { ch0:0, ch1:1 },
    Ftm1Channels { ch0:0, ch1:1 },
    scgc6, ftm1
);

// Only mk20d7 has FTM2 (2 channels)
#[cfg(feature = "mk20d7")]
ftm_impl!(pac::Ftm2, Ftm2,
    Ftm2Parts { ch0:0, ch1:1 },
    Ftm2Channels { ch0:0, ch1:1 },
    scgc3, ftm2
);

// =====================================================================
// Channel Mode Enums
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

// =====================================================================
// Output Compare enums
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
