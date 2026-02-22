use core::marker::PhantomData;

use crate::clocks::Clocks;
use crate::pac;
use crate::time::Hertz;

// ----- Instance Markers -----

/// Marker type for FTM0.
pub struct Ftm0;

/// Marker type for FTM1.
pub struct Ftm1;

/// Marker type for FTM2 (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Ftm2;

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
fn calc_prescaler(bus_clk: u32, target_freq: u32) -> (u8, u16) {
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

                let ftm = unsafe { &*<$PacType>::PTR };

                // 1. Disable counter (CLKS=None)
                ftm.sc().write(|w| w.clks().none());

                // 2. Disable write protection
                ftm.mode().write(|w| w.wpdis()._1());

                // 3. Set counter initial value to 0
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
