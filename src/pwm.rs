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

/// Set a single channel's OUTINIT bit (initial output value on MODE.INIT trigger).
fn set_outinit_bit<FTM: sealed::FtmInstance>(ch: u8, high: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.outinit().modify(|_, w| match (ch, high) {
        (0, false) => w.ch0oi().low(),
        (0, true) => w.ch0oi().high(),
        (1, false) => w.ch1oi().low(),
        (1, true) => w.ch1oi().high(),
        (2, false) => w.ch2oi().low(),
        (2, true) => w.ch2oi().high(),
        (3, false) => w.ch3oi().low(),
        (3, true) => w.ch3oi().high(),
        (4, false) => w.ch4oi().low(),
        (4, true) => w.ch4oi().high(),
        (5, false) => w.ch5oi().low(),
        (5, true) => w.ch5oi().high(),
        (6, false) => w.ch6oi().low(),
        (6, true) => w.ch6oi().high(),
        (7, false) => w.ch7oi().low(),
        (7, true) => w.ch7oi().high(),
        _ => unreachable!(),
    });
}

/// Set a single channel's POL bit (hardware output polarity inversion).
fn set_pol_bit<FTM: sealed::FtmInstance>(ch: u8, active_low: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.pol().modify(|_, w| match (ch, active_low) {
        (0, false) => w.pol0().active_high(),
        (0, true) => w.pol0().active_low(),
        (1, false) => w.pol1().active_high(),
        (1, true) => w.pol1().active_low(),
        (2, false) => w.pol2().active_high(),
        (2, true) => w.pol2().active_low(),
        (3, false) => w.pol3().active_high(),
        (3, true) => w.pol3().active_low(),
        (4, false) => w.pol4().active_high(),
        (4, true) => w.pol4().active_low(),
        (5, false) => w.pol5().active_high(),
        (5, true) => w.pol5().active_low(),
        (6, false) => w.pol6().active_high(),
        (6, true) => w.pol6().active_low(),
        (7, false) => w.pol7().active_high(),
        (7, true) => w.pol7().active_low(),
        _ => unreachable!(),
    });
}

/// Set a pair's COMBINE bit (combined mode enable).
///
/// When set, channels (2*pair) and (2*pair+1) operate as a combined pair
/// where CnV controls the leading edge and C(n+1)V controls the trailing edge.
/// Requires FTMEN=1 and WPDIS=1. Ref manual §36.4.15 (K20P64M72SF1RM).
fn set_combine_bit<FTM: sealed::FtmInstance>(pair: u8, enable: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.combine().modify(|_, w| match (pair, enable) {
        (0, false) => w.combine0().disabled(),
        (0, true) => w.combine0().enabled(),
        (1, false) => w.combine1().disabled(),
        (1, true) => w.combine1().enabled(),
        (2, false) => w.combine2().disabled(),
        (2, true) => w.combine2().enabled(),
        (3, false) => w.combine3().disabled(),
        (3, true) => w.combine3().enabled(),
        _ => unreachable!(),
    });
}

/// Set a pair's COMP bit (complementary output).
///
/// When set, channel (2*pair+1) output is the complement of channel (2*pair).
/// This provides the inverted gate drive signal needed for half-bridge and
/// H-bridge topologies. Ref manual §36.4.15 (K20P64M72SF1RM).
fn set_comp_bit<FTM: sealed::FtmInstance>(pair: u8, enable: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.combine().modify(|_, w| match (pair, enable) {
        (0, false) => w.comp0().disabled(),
        (0, true) => w.comp0().enabled(),
        (1, false) => w.comp1().disabled(),
        (1, true) => w.comp1().enabled(),
        (2, false) => w.comp2().disabled(),
        (2, true) => w.comp2().enabled(),
        (3, false) => w.comp3().disabled(),
        (3, true) => w.comp3().enabled(),
        _ => unreachable!(),
    });
}

/// Set a pair's DTEN bit (dead-time insertion enable).
///
/// When set, dead-time is inserted at transitions between the complementary
/// outputs of the pair. The dead-time duration is configured timer-wide via
/// the DEADTIME register. Prevents shoot-through in half-bridge drivers.
/// Ref manual §36.4.15 (K20P64M72SF1RM).
fn set_dten_bit<FTM: sealed::FtmInstance>(pair: u8, enable: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.combine().modify(|_, w| match (pair, enable) {
        (0, false) => w.dten0().disabled(),
        (0, true) => w.dten0().enabled(),
        (1, false) => w.dten1().disabled(),
        (1, true) => w.dten1().enabled(),
        (2, false) => w.dten2().disabled(),
        (2, true) => w.dten2().enabled(),
        (3, false) => w.dten3().disabled(),
        (3, true) => w.dten3().enabled(),
        _ => unreachable!(),
    });
}

/// Set a pair's SYNCEN bit (PWM synchronization enable).
///
/// When set, the pair's CnV and C(n+1)V registers are updated from their
/// write buffers at the configured loading point (SYNC.CNTMIN/CNTMAX).
/// Required for glitch-free runtime updates in combined mode.
/// Ref manual §36.4.15 (K20P64M72SF1RM).
fn set_syncen_bit<FTM: sealed::FtmInstance>(pair: u8, enable: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.combine().modify(|_, w| match (pair, enable) {
        (0, false) => w.syncen0().disabled(),
        (0, true) => w.syncen0().enabled(),
        (1, false) => w.syncen1().disabled(),
        (1, true) => w.syncen1().enabled(),
        (2, false) => w.syncen2().disabled(),
        (2, true) => w.syncen2().enabled(),
        (3, false) => w.syncen3().disabled(),
        (3, true) => w.syncen3().enabled(),
        _ => unreachable!(),
    });
}

/// Set a pair's inversion bit in INVCTRL.
///
/// Controls the INVnEN bit for the given pair. When set, the pair's outputs
/// are swapped. With SYNCONF.INVC=0 (default), takes effect at the next
/// system clock edge (immediately). Ref manual §36.4.22 (K20P64M72SF1RM).
fn set_inv_bit<FTM: sealed::FtmInstance>(pair: u8, enable: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.invctrl().modify(|_, w| match (pair, enable) {
        (0, false) => w.inv0en().disabled(),
        (0, true) => w.inv0en().enabled(),
        (1, false) => w.inv1en().disabled(),
        (1, true) => w.inv1en().enabled(),
        (2, false) => w.inv2en().disabled(),
        (2, true) => w.inv2en().enabled(),
        (3, false) => w.inv3en().disabled(),
        (3, true) => w.inv3en().enabled(),
        _ => unreachable!(),
    });
}

/// Set a single channel's SWOCTRL bits (software output control).
///
/// Controls both the CHnOC (enable) and CHnOCV (forced value) bits.
/// When `enable` is true, the channel output is forced to the level
/// specified by `high`. When `enable` is false, the channel returns
/// to normal operation. Ref manual §36.4.19 (K20P64M72SF1RM).
fn set_swoctrl<FTM: sealed::FtmInstance>(ch: u8, enable: bool, high: bool) {
    let ftm = ftm_regs::<FTM>();
    ftm.swoctrl().modify(|_, w| match (ch, enable, high) {
        (0, true, true) => w.ch0oc().enabled().ch0ocv().force_high(),
        (0, true, false) => w.ch0oc().enabled().ch0ocv().force_low(),
        (0, false, _) => w.ch0oc().disabled(),
        (1, true, true) => w.ch1oc().enabled().ch1ocv().force_high(),
        (1, true, false) => w.ch1oc().enabled().ch1ocv().force_low(),
        (1, false, _) => w.ch1oc().disabled(),
        (2, true, true) => w.ch2oc().enabled().ch2ocv().force_high(),
        (2, true, false) => w.ch2oc().enabled().ch2ocv().force_low(),
        (2, false, _) => w.ch2oc().disabled(),
        (3, true, true) => w.ch3oc().enabled().ch3ocv().force_high(),
        (3, true, false) => w.ch3oc().enabled().ch3ocv().force_low(),
        (3, false, _) => w.ch3oc().disabled(),
        (4, true, true) => w.ch4oc().enabled().ch4ocv().force_high(),
        (4, true, false) => w.ch4oc().enabled().ch4ocv().force_low(),
        (4, false, _) => w.ch4oc().disabled(),
        (5, true, true) => w.ch5oc().enabled().ch5ocv().force_high(),
        (5, true, false) => w.ch5oc().enabled().ch5ocv().force_low(),
        (5, false, _) => w.ch5oc().disabled(),
        (6, true, true) => w.ch6oc().enabled().ch6ocv().force_high(),
        (6, true, false) => w.ch6oc().enabled().ch6ocv().force_low(),
        (6, false, _) => w.ch6oc().disabled(),
        (7, true, true) => w.ch7oc().enabled().ch7ocv().force_high(),
        (7, true, false) => w.ch7oc().enabled().ch7ocv().force_low(),
        (7, false, _) => w.ch7oc().disabled(),
        _ => unreachable!(),
    });
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
    ///
    /// **Note:** Center-aligned mode (CPWM) requires CNTIN=0x0000 and
    /// is mutually exclusive with Combined mode (which requires
    /// CPWMS=0). Ref manual §36.4.7 (K20P64M72SF1RM).
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

    // --- Output initialization (OUTINIT + MODE.INIT) ---

    /// Trigger the output initialization sequence.
    ///
    /// Sets MODE.INIT, which loads all OUTINIT values into the channel
    /// outputs simultaneously. The INIT bit is self-clearing (hardware
    /// clears it after the initialization completes).
    ///
    /// **Note:** The initialization feature must be used only in Combine
    /// mode and with the FTM counter disabled (CLKS=None). Ref manual
    /// §36.4.18 (K20P64M72SF1RM).
    pub fn trigger_output_init(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.mode().modify(|_, w| w.init().set_bit());
    }

    /// Bulk-set OUTINIT for all channels and trigger initialization.
    ///
    /// Each bit in `channel_mask` sets the initial output level for the
    /// corresponding channel: bit N high = channel N initializes high,
    /// bit N low = channel N initializes low. After writing OUTINIT,
    /// MODE.INIT is set to apply the values.
    ///
    /// **Note:** The initialization feature must be used only in Combine
    /// mode and with the FTM counter disabled (CLKS=None). Ref manual
    /// §36.4.18 (K20P64M72SF1RM).
    pub fn init_outputs(&mut self, channel_mask: u8) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: only the low 8 bits of OUTINIT are defined; channel_mask is u8.
        ftm.outinit().write(|w| unsafe { w.bits(channel_mask as u32) });
        ftm.mode().modify(|_, w| w.init().set_bit());
    }

    // --- Dead-time configuration (timer-wide DEADTIME register) ---

    /// Configure the dead-time prescaler and value.
    ///
    /// Dead-time is inserted between complementary outputs of combined
    /// channel pairs (when DTEN=1). The dead-time duration is:
    ///
    ///   `dead_time = dtval × (bus_clk_period × prescaler_divisor)`
    ///
    /// `dtval` is masked to 6 bits (0-63). The dead-time register is
    /// timer-wide — all pairs share the same dead-time duration. Per-pair
    /// dead-time enable is controlled via [`FtmChannelPair::enable_deadtime`].
    ///
    /// **Note:** The dead-time feature must be used only in Combine and
    /// Complementary modes. Ref manual §36.4.16 (K20P64M72SF1RM).
    pub fn set_deadtime(&mut self, prescaler: DeadtimePrescaler, dtval: u8) {
        let ftm = ftm_regs::<FTM>();
        ftm.deadtime().write(|w| {
            // SAFETY: dtval is a 6-bit field; value is masked to 0x3F.
            let w = unsafe { w.dtval().bits(dtval & 0x3F) };
            match prescaler {
                DeadtimePrescaler::Div1 => w.dtps()._0x(),
                DeadtimePrescaler::Div4 => w.dtps()._10(),
                DeadtimePrescaler::Div16 => w.dtps()._11(),
            }
        });
    }

    /// Read the current dead-time prescaler setting.
    pub fn deadtime_prescaler(&self) -> DeadtimePrescaler {
        let ftm = ftm_regs::<FTM>();
        let dtps = ftm.deadtime().read().dtps();
        if dtps.is_10() {
            DeadtimePrescaler::Div4
        } else if dtps.is_11() {
            DeadtimePrescaler::Div16
        } else {
            DeadtimePrescaler::Div1
        }
    }

    /// Read the current dead-time value (DTVAL, 6 bits).
    pub fn deadtime_value(&self) -> u8 {
        let ftm = ftm_regs::<FTM>();
        ftm.deadtime().read().dtval().bits()
    }

    // --- Sync control ---

    /// Trigger a software synchronization.
    ///
    /// Sets SYNC.SWSYNC=1, which flushes double-buffered registers
    /// (MOD, CNTIN, CnV) to their active values at the next loading
    /// point. The SWSYNC bit is auto-cleared by hardware.
    ///
    /// Use this after updating combined-mode edge values via
    /// [`FtmChannelPair::set_edges`] for glitch-free runtime updates.
    ///
    /// **Note:** PWM synchronization must be used only in Combine mode.
    /// Ref manual §36.4.11, §36.4.21 (K20P64M72SF1RM).
    pub fn software_sync(&mut self) {
        let ftm = ftm_regs::<FTM>();
        ftm.sync().modify(|_, w| w.swsync()._1());
    }

    /// Configure PWM synchronization loading points.
    ///
    /// Controls when double-buffered register values (CnV, MOD, CNTIN)
    /// are loaded into their active registers:
    /// - `at_min`: load when the counter reaches CNTIN (period start)
    /// - `at_max`: load when the counter reaches MOD (period end)
    ///
    /// At least one should be enabled for combined-mode operation.
    ///
    /// **Note:** PWM synchronization must be used only in Combine mode.
    /// Ref manual §36.4.11, §36.4.21 (K20P64M72SF1RM).
    pub fn set_sync_loading_points(&mut self, at_min: bool, at_max: bool) {
        let ftm = ftm_regs::<FTM>();
        ftm.sync().modify(|_, w| {
            let w = if at_min { w.cntmin()._1() } else { w.cntmin()._0() };
            if at_max { w.cntmax()._1() } else { w.cntmax()._0() }
        });
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
// Output Level (OUTINIT / SWOCTRL)
// =====================================================================

/// Logic level for channel output initialization (OUTINIT) and
/// software output control (SWOCTRL).
///
/// Used with [`FtmChannel::set_output_init`] and [`FtmChannel::force_output`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OutputLevel {
    /// Output driven low.
    Low,
    /// Output driven high.
    High,
}

// =====================================================================
// Output Polarity (POL register)
// =====================================================================

/// Hardware output polarity (POL register).
///
/// Controls final-stage inversion of the FTM channel output. This is
/// distinct from [`PwmPolarity`] which shapes the PWM waveform via
/// the CnSC ELS bits.
///
/// Used with [`FtmChannel::set_polarity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Polarity {
    /// Channel output is active-high (no inversion).
    ActiveHigh,
    /// Channel output is active-low (inverted).
    ActiveLow,
}

// =====================================================================
// Dead-Time Prescaler (DEADTIME.DTPS)
// =====================================================================

/// Dead-time insertion prescaler (DEADTIME.DTPS field).
///
/// Selects the clock divider applied to the system (bus) clock before
/// the dead-time counter. The resulting dead-time duration is:
///
///   `dead_time = DTVAL × (bus_clk_period × prescaler_divisor)`
///
/// For example, at 36 MHz bus clock with `Div1` and DTVAL=10:
///   `10 × (1/36 MHz) × 1 ≈ 278 ns`
///
/// Ref manual §36.4.16 (K20P64M72SF1RM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DeadtimePrescaler {
    /// Divide by 1 (DTPS = 0x).
    Div1,
    /// Divide by 4 (DTPS = 10).
    Div4,
    /// Divide by 16 (DTPS = 11).
    Div16,
}

// =====================================================================
// Pair Inversion (INVCTRL)
// =====================================================================

/// Output inversion state for a combined channel pair.
///
/// Controls the INVCTRL register's INVnEN bit for a channel pair.
/// When inverted, the pair's outputs are swapped. Takes effect
/// immediately by default (SYNCONF.INVC=0).
///
/// Ref manual §36.4.22 (K20P64M72SF1RM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PairInversion {
    /// Normal output (no inversion).
    Normal,
    /// Inverted output (pair outputs swapped).
    Inverted,
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
    ///
    /// **Note:** EPWM mode must be used only with CNTIN=0x0000.
    /// Ref manual §36.4.6 (K20P64M72SF1RM).
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
    /// Works with both edge-aligned (EPWM) and center-aligned (CPWM) modes.
    /// Both require CNTIN=0x0000. Ref manual §36.4.6, §36.4.7
    /// (K20P64M72SF1RM).
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
    ///
    /// **Note:** Output Compare mode must be used only with CNTIN=0x0000.
    /// Ref manual §36.4.6 (K20P64M72SF1RM).
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
    ///
    /// **Note:** Input Capture mode must be used only with CNTIN=0x0000.
    /// Ref manual §36.4.5 (K20P64M72SF1RM).
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
    /// CHF is cleared by reading CnSC while CHF is set, then writing 0
    /// to CHF (bit 7). The SVD marks CHF as read-only, so `modify()`
    /// would write back the read value (CHF=1), which does **not** clear
    /// CHF — "writing a 1 to CHF has no effect" (K20 ref manual §36.3.6).
    /// We use raw bits to force bit 7 to 0 in the write-back.
    pub fn clear_flag(&mut self) {
        let ftm = ftm_regs::<FTM>();
        let csc = ftm.csc(CH as usize);
        let bits = csc.read().bits();
        // SAFETY: we preserve all read-write field values from the read;
        // only CHF (bit 7, read-only per SVD but clearable per refman)
        // is forced to 0.
        csc.write(|w| unsafe { w.bits(bits & !(1 << 7)) });
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

    // --- Output initialization (OUTINIT) ---

    /// Set the initial output value for this channel.
    ///
    /// The configured level is applied to the channel output when
    /// [`FtmTimer::trigger_output_init`] sets MODE.INIT.
    ///
    /// **Note:** The initialization feature must be used only in Combine
    /// mode and with the FTM counter disabled (CLKS=None). Ref manual
    /// §36.4.18 (K20P64M72SF1RM).
    pub fn set_output_init(&mut self, level: OutputLevel) {
        set_outinit_bit::<FTM>(CH, matches!(level, OutputLevel::High));
    }

    /// Read the current OUTINIT value for this channel.
    pub fn output_init(&self) -> OutputLevel {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.outinit().read();
        let high = match CH {
            0 => r.ch0oi().is_high(),
            1 => r.ch1oi().is_high(),
            2 => r.ch2oi().is_high(),
            3 => r.ch3oi().is_high(),
            4 => r.ch4oi().is_high(),
            5 => r.ch5oi().is_high(),
            6 => r.ch6oi().is_high(),
            7 => r.ch7oi().is_high(),
            _ => unreachable!(),
        };
        if high { OutputLevel::High } else { OutputLevel::Low }
    }

    // --- Output polarity (POL register) ---

    /// Set hardware output polarity for this channel.
    ///
    /// `ActiveLow` inverts the final output stage. This is distinct from
    /// [`set_pwm_polarity`](Self::set_pwm_polarity) which controls the
    /// CnSC ELS bits (PWM waveform shape).
    ///
    /// **Note:** The polarity control must be used only in Combine mode.
    /// Ref manual §36.4.12 (K20P64M72SF1RM).
    pub fn set_polarity(&mut self, pol: Polarity) {
        set_pol_bit::<FTM>(CH, matches!(pol, Polarity::ActiveLow));
    }

    /// Read the current hardware output polarity for this channel.
    pub fn polarity(&self) -> Polarity {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.pol().read();
        let active_low = match CH {
            0 => r.pol0().is_active_low(),
            1 => r.pol1().is_active_low(),
            2 => r.pol2().is_active_low(),
            3 => r.pol3().is_active_low(),
            4 => r.pol4().is_active_low(),
            5 => r.pol5().is_active_low(),
            6 => r.pol6().is_active_low(),
            7 => r.pol7().is_active_low(),
            _ => unreachable!(),
        };
        if active_low { Polarity::ActiveLow } else { Polarity::ActiveHigh }
    }

    // --- Software output control (SWOCTRL) ---

    /// Force this channel's output to a specific logic level.
    ///
    /// Enables software output control (CHnOC) and sets the forced
    /// value (CHnOCV) in a single register write. The channel output
    /// is driven to `level` regardless of the PWM/OC waveform.
    ///
    /// Requires FTMEN=1 (set by [`FtmExt::split`]/[`FtmExt::pwm`]).
    ///
    /// **Note:** The software output control feature must be used only
    /// in Combine mode. SWOCTRL bits are updated at the next loading
    /// point when SYNCMODE=0, or immediately when SYNCMODE=1.
    /// Ref manual §36.4.14, §36.4.19 (K20P64M72SF1RM).
    pub fn force_output(&mut self, level: OutputLevel) {
        let high = matches!(level, OutputLevel::High);
        set_swoctrl::<FTM>(CH, true, high);
    }

    /// Release software output control, returning to normal operation.
    ///
    /// Clears CHnOC for this channel. The output resumes being driven
    /// by the configured PWM/OC mode. Combine mode only (ref manual
    /// §36.4.14).
    pub fn release_output(&mut self) {
        set_swoctrl::<FTM>(CH, false, false);
    }

    /// Check if software output control is active for this channel.
    pub fn is_output_forced(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.swoctrl().read();
        match CH {
            0 => r.ch0oc().is_enabled(),
            1 => r.ch1oc().is_enabled(),
            2 => r.ch2oc().is_enabled(),
            3 => r.ch3oc().is_enabled(),
            4 => r.ch4oc().is_enabled(),
            5 => r.ch5oc().is_enabled(),
            6 => r.ch6oc().is_enabled(),
            7 => r.ch7oc().is_enabled(),
            _ => unreachable!(),
        }
    }

    /// Read the forced output value if software control is active.
    ///
    /// Returns `Some(level)` if CHnOC is set, `None` otherwise.
    pub fn forced_output(&self) -> Option<OutputLevel> {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.swoctrl().read();
        let enabled = match CH {
            0 => r.ch0oc().is_enabled(),
            1 => r.ch1oc().is_enabled(),
            2 => r.ch2oc().is_enabled(),
            3 => r.ch3oc().is_enabled(),
            4 => r.ch4oc().is_enabled(),
            5 => r.ch5oc().is_enabled(),
            6 => r.ch6oc().is_enabled(),
            7 => r.ch7oc().is_enabled(),
            _ => unreachable!(),
        };
        if !enabled {
            return None;
        }
        let high = match CH {
            0 => r.ch0ocv().is_force_high(),
            1 => r.ch1ocv().is_force_high(),
            2 => r.ch2ocv().is_force_high(),
            3 => r.ch3ocv().is_force_high(),
            4 => r.ch4ocv().is_force_high(),
            5 => r.ch5ocv().is_force_high(),
            6 => r.ch6ocv().is_force_high(),
            7 => r.ch7ocv().is_force_high(),
            _ => unreachable!(),
        };
        Some(if high { OutputLevel::High } else { OutputLevel::Low })
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

// =====================================================================
// FtmChannelPair — combined mode channel pair
// =====================================================================

/// A pair of adjacent FTM channels operating in combined mode.
///
/// Combined mode pairs channels (n, n+1) where:
/// - CnV controls the **leading edge** (output asserts)
/// - C(n+1)V controls the **trailing edge** (output deasserts)
///
/// This enables complementary PWM with dead-time insertion, essential for
/// motor control (H-bridges, 3-phase inverters) and other applications
/// requiring non-overlapping complementary outputs.
///
/// Created by calling [`FtmChannel::into_combined`] on an even channel,
/// consuming both the even and odd channel handles. Can be released back
/// to independent channels via [`into_channels`](Self::into_channels).
///
/// **Hardware constraint:** Combined mode requires CPWMS=0 (edge-aligned
/// counting). It is mutually exclusive with center-aligned mode.
///
/// # Synchronization
///
/// CnV/C(n+1)V writes are double-buffered. Call
/// [`FtmTimer::software_sync`] to commit pending values at the next
/// loading point (configured via [`FtmTimer::set_sync_loading_points`]).
///
/// Ref manual §36.4.15 (COMBINE), §36.4.16 (DEADTIME), §36.4.22 (INVCTRL)
/// (K20P64M72SF1RM).
pub struct FtmChannelPair<FTM, const PAIR: u8> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: sealed::FtmInstance, const PAIR: u8> FtmChannelPair<FTM, PAIR> {
    /// Even channel index for this pair.
    const EVEN: usize = (PAIR as usize) * 2;
    /// Odd channel index for this pair.
    const ODD: usize = (PAIR as usize) * 2 + 1;

    /// Initialize combined mode for this channel pair.
    ///
    /// Configures both channels for combined PWM, enables enhanced sync
    /// buffering (SYNCONF.SWWRBUF), sets the loading point to counter
    /// minimum (SYNC.CNTMIN), and enables the COMBINE bit.
    ///
    /// Called by the macro-generated `into_combined()` method.
    fn new_init() -> Self {
        let ftm = ftm_regs::<FTM>();

        // Configure CnSC for both channels: MSB=1, ELSB=1 (high-true combined PWM)
        ftm.csc(Self::EVEN).write(|w| w.msb().set_bit().elsb().set_bit());
        ftm.csc(Self::ODD).write(|w| w.msb().set_bit().elsb().set_bit());

        // Initialize CV registers to 0 (0% duty)
        // SAFETY: val is a 16-bit field; 0 fits.
        ftm.cv(Self::EVEN).write(|w| unsafe { w.val().bits(0) });
        ftm.cv(Self::ODD).write(|w| unsafe { w.val().bits(0) });

        // Enable CV double-buffering via software trigger.
        // SYNCONF.SWWRBUF=1: software trigger flushes MOD/CNTIN/CV write buffers.
        // SYNCONF.INVC is left at 0 so INVCTRL updates take effect immediately
        // (at every system clock edge). This is the right default since inversion
        // is typically configured once at init. Users needing synchronized inversion
        // changes can set SYNCONF.INVC=1 via the PAC directly.
        // Ref manual §36.4.27.
        ftm.synconf().modify(|_, w| w.swwrbuf()._1());

        // Set loading point: update at counter minimum (period boundary).
        // Ref manual §36.4.21.
        ftm.sync().modify(|_, w| w.cntmin()._1());

        // Enable PWM sync for this pair, then enable combined mode.
        // SYNCEN must be set before COMBINE for proper synchronization.
        // Ref manual §36.4.15.
        set_syncen_bit::<FTM>(PAIR, true);
        set_combine_bit::<FTM>(PAIR, true);

        FtmChannelPair { _ftm: PhantomData }
    }

    // --- Edge control ---

    /// Set the leading edge value (CnV for the even channel).
    ///
    /// The output asserts (goes active) when the counter reaches this value.
    /// Double-buffered: takes effect at the next loading point after a
    /// software sync trigger. Ref manual §36.4.15.
    pub fn set_leading_edge(&mut self, val: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: val is a 16-bit field; val is u16.
        ftm.cv(Self::EVEN).write(|w| unsafe { w.val().bits(val) });
    }

    /// Set the trailing edge value (C(n+1)V for the odd channel).
    ///
    /// The output deasserts (goes inactive) when the counter reaches this value.
    /// Double-buffered: takes effect at the next loading point after a
    /// software sync trigger. Ref manual §36.4.15.
    pub fn set_trailing_edge(&mut self, val: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: val is a 16-bit field; val is u16.
        ftm.cv(Self::ODD).write(|w| unsafe { w.val().bits(val) });
    }

    /// Set both leading and trailing edge values.
    ///
    /// Convenience method for asymmetric PWM. The active pulse spans from
    /// `leading` to `trailing` counter values. For example, `set_edges(100, 500)`
    /// produces a pulse from count 100 to count 500.
    ///
    /// Double-buffered: call [`FtmTimer::software_sync`] to commit.
    pub fn set_edges(&mut self, leading: u16, trailing: u16) {
        let ftm = ftm_regs::<FTM>();
        // SAFETY: val is a 16-bit field; values are u16.
        ftm.cv(Self::EVEN).write(|w| unsafe { w.val().bits(leading) });
        ftm.cv(Self::ODD).write(|w| unsafe { w.val().bits(trailing) });
    }

    /// Read the current leading edge value.
    pub fn leading_edge(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.cv(Self::EVEN).read().val().bits()
    }

    /// Read the current trailing edge value.
    pub fn trailing_edge(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.cv(Self::ODD).read().val().bits()
    }

    /// Set symmetric duty cycle (leading edge at 0).
    ///
    /// Equivalent to `set_edges(0, duty)`. The active pulse spans from
    /// counter value 0 to `duty`.
    pub fn set_duty(&mut self, duty: u16) {
        self.set_edges(0, duty);
    }

    /// Read the MOD register (maximum duty value).
    ///
    /// The duty cycle as a fraction of the period is `duty / max_duty()`.
    pub fn max_duty(&self) -> u16 {
        let ftm = ftm_regs::<FTM>();
        ftm.mod_().read().mod_().bits()
    }

    // --- Complementary output (COMP bit in COMBINE) ---

    /// Enable complementary output for this pair.
    ///
    /// When enabled, channel (n+1) output is the hardware complement of
    /// channel (n). This provides the inverted gate drive signal needed
    /// for half-bridge and H-bridge topologies.
    /// Ref manual §36.4.15 (K20P64M72SF1RM).
    pub fn enable_complementary(&mut self) {
        set_comp_bit::<FTM>(PAIR, true);
    }

    /// Disable complementary output (both channels driven independently).
    pub fn disable_complementary(&mut self) {
        set_comp_bit::<FTM>(PAIR, false);
    }

    /// Check if complementary output is enabled.
    pub fn is_complementary(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.combine().read();
        match PAIR {
            0 => r.comp0().is_enabled(),
            1 => r.comp1().is_enabled(),
            2 => r.comp2().is_enabled(),
            3 => r.comp3().is_enabled(),
            _ => unreachable!(),
        }
    }

    // --- Dead-time enable (DTEN bit in COMBINE) ---

    /// Enable dead-time insertion for this pair.
    ///
    /// Dead-time is inserted at transitions between the complementary outputs,
    /// preventing shoot-through in half-bridge drivers. The dead-time duration
    /// is configured timer-wide via [`FtmTimer::set_deadtime`].
    ///
    /// Typically used together with [`enable_complementary`](Self::enable_complementary).
    /// Ref manual §36.4.15, §36.4.16 (K20P64M72SF1RM).
    pub fn enable_deadtime(&mut self) {
        set_dten_bit::<FTM>(PAIR, true);
    }

    /// Disable dead-time insertion.
    pub fn disable_deadtime(&mut self) {
        set_dten_bit::<FTM>(PAIR, false);
    }

    /// Check if dead-time insertion is enabled.
    pub fn is_deadtime_enabled(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.combine().read();
        match PAIR {
            0 => r.dten0().is_enabled(),
            1 => r.dten1().is_enabled(),
            2 => r.dten2().is_enabled(),
            3 => r.dten3().is_enabled(),
            _ => unreachable!(),
        }
    }

    // --- PWM synchronization (SYNCEN bit in COMBINE) ---

    /// Enable PWM synchronization for this pair.
    ///
    /// When enabled, CnV and C(n+1)V writes go to a buffer and are
    /// transferred to the active registers only at the configured
    /// loading point after a [`FtmTimer::software_sync`] trigger.
    /// This provides glitch-free updates for motor control applications.
    ///
    /// Enabled by default in [`into_combined`](FtmChannel::into_combined).
    /// Ref manual §36.4.15 (K20P64M72SF1RM).
    pub fn enable_sync(&mut self) {
        set_syncen_bit::<FTM>(PAIR, true);
    }

    /// Disable PWM synchronization for this pair.
    ///
    /// When disabled, CnV writes update via normal FTM double-buffering
    /// (latched at counter overflow) without requiring a software sync
    /// trigger. This is useful for DMA-driven applications where each
    /// CnV write must take effect on the very next PWM period.
    ///
    /// Ref manual §36.4.15 (K20P64M72SF1RM).
    pub fn disable_sync(&mut self) {
        set_syncen_bit::<FTM>(PAIR, false);
    }

    /// Check if PWM synchronization is enabled for this pair.
    pub fn is_sync_enabled(&self) -> bool {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.combine().read();
        match PAIR {
            0 => r.syncen0().is_enabled(),
            1 => r.syncen1().is_enabled(),
            2 => r.syncen2().is_enabled(),
            3 => r.syncen3().is_enabled(),
            _ => unreachable!(),
        }
    }

    // --- Output inversion (INVCTRL) ---

    /// Set the output inversion state for this pair.
    ///
    /// When [`PairInversion::Inverted`], the pair's outputs are swapped.
    /// By default (SYNCONF.INVC=0), takes effect at the next system
    /// clock edge (effectively immediately). Ref manual §36.4.22
    /// (K20P64M72SF1RM).
    pub fn set_inversion(&mut self, inv: PairInversion) {
        set_inv_bit::<FTM>(PAIR, matches!(inv, PairInversion::Inverted));
    }

    /// Read the current inversion state.
    pub fn inversion(&self) -> PairInversion {
        let ftm = ftm_regs::<FTM>();
        let r = ftm.invctrl().read();
        let inverted = match PAIR {
            0 => r.inv0en().is_enabled(),
            1 => r.inv1en().is_enabled(),
            2 => r.inv2en().is_enabled(),
            3 => r.inv3en().is_enabled(),
            _ => unreachable!(),
        };
        if inverted { PairInversion::Inverted } else { PairInversion::Normal }
    }

    // --- PWM polarity (CnSC ELS bits for even channel) ---

    /// Set the PWM polarity for the combined pair.
    ///
    /// Controls the ELS bits on the even channel's CnSC register.
    /// `HighTrue`: output is high during the active period (ELSB=1, ELSA=0).
    /// `LowTrue`: output is low during the active period (ELSB=0, ELSA=1).
    ///
    /// Ref manual §36.4.6 (K20P64M72SF1RM).
    pub fn set_pwm_polarity(&mut self, pol: PwmPolarity) {
        let ftm = ftm_regs::<FTM>();
        ftm.csc(Self::EVEN).write(|w| {
            let w = w.msb().set_bit();
            match pol {
                PwmPolarity::HighTrue => w.elsb().set_bit(),
                PwmPolarity::LowTrue => w.elsa().set_bit(),
            }
        });
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

                // Disable write protection first (FTMEN still 0).
                // Per ref manual §36.4.27, SYNCONF should be written while
                // FTMEN=0 so enhanced sync mode is latched before FTM
                // features are enabled.
                ftm.mode().write(|w| w.wpdis()._1());
                ftm.synconf().write(|w| w.syncmode()._1());
                ftm.mode().modify(|_, w| w.ftmen()._1());

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

                // 2. Disable write protection, enable enhanced sync, then FTMEN
                ftm.mode().write(|w| w.wpdis()._1());
                ftm.synconf().write(|w| w.syncmode()._1());
                ftm.mode().modify(|_, w| w.ftmen()._1());

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
// Combined-mode pair constructors (into_combined / into_channels)
// =====================================================================

/// Generates `into_combined()` on the even `FtmChannel` and `into_channels()`
/// on the resulting `FtmChannelPair`, linking a specific (even, odd) channel
/// pair to a PAIR index without requiring unstable `generic_const_exprs`.
macro_rules! ftm_pair_impl {
    ($Instance:ty, pair: $PAIR:literal, even: $EVEN:literal, odd: $ODD:literal) => {
        impl FtmChannel<$Instance, $EVEN> {
            /// Consume this channel and its partner to create a combined-mode pair.
            ///
            /// Configures channels for combined PWM: the even channel's CnV
            /// controls the leading edge, the odd channel's C(n+1)V controls
            /// the trailing edge. Both channels start at 0% duty.
            ///
            /// After creation, call [`FtmChannelPair::enable_complementary`]
            /// and [`FtmChannelPair::enable_deadtime`] as needed.
            ///
            /// Ref manual §36.4.15 (K20P64M72SF1RM).
            pub fn into_combined(
                self, _partner: FtmChannel<$Instance, $ODD>,
            ) -> FtmChannelPair<$Instance, $PAIR> {
                FtmChannelPair::new_init()
            }
        }
        impl FtmChannelPair<$Instance, $PAIR> {
            /// Release the combined pair back into two independent channels.
            ///
            /// Clears COMBINE, COMP, DTEN, and SYNCEN for this pair.
            /// The returned channels are in an unconfigured state — call
            /// [`FtmChannel::set_pwm`] or another mode method to resume use.
            pub fn into_channels(self) -> (
                FtmChannel<$Instance, $EVEN>,
                FtmChannel<$Instance, $ODD>,
            ) {
                set_combine_bit::<$Instance>($PAIR, false);
                set_comp_bit::<$Instance>($PAIR, false);
                set_dten_bit::<$Instance>($PAIR, false);
                set_syncen_bit::<$Instance>($PAIR, false);
                (FtmChannel { _ftm: PhantomData }, FtmChannel { _ftm: PhantomData })
            }
        }
    };
}

// FTM0: 4 pairs (8 channels)
ftm_pair_impl!(Ftm0, pair: 0, even: 0, odd: 1);
ftm_pair_impl!(Ftm0, pair: 1, even: 2, odd: 3);
ftm_pair_impl!(Ftm0, pair: 2, even: 4, odd: 5);
ftm_pair_impl!(Ftm0, pair: 3, even: 6, odd: 7);

// FTM1: 1 pair (2 channels)
ftm_pair_impl!(Ftm1, pair: 0, even: 0, odd: 1);

// FTM2: 1 pair (mk20d7 only)
#[cfg(feature = "mk20d7")]
ftm_pair_impl!(Ftm2, pair: 0, even: 0, odd: 1);

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
