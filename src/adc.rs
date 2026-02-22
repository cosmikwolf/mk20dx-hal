use core::marker::PhantomData;

use crate::clocks::Clocks;
use crate::pac;

// ----- Instance Markers -----

/// Marker type for ADC0.
pub struct Adc0;

/// Marker type for ADC1 (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Adc1;

// ----- Configuration Enums -----

/// ADC conversion resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// 8-bit conversion (9-bit in differential mode).
    Bits8,
    /// 10-bit conversion (11-bit in differential mode).
    Bits10,
    /// 12-bit conversion (13-bit in differential mode).
    Bits12,
    /// 16-bit conversion.
    Bits16,
}

/// Hardware averaging configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Averaging {
    /// Hardware averaging disabled.
    Disabled,
    /// Average 4 samples.
    Avg4,
    /// Average 8 samples.
    Avg8,
    /// Average 16 samples.
    Avg16,
    /// Average 32 samples.
    Avg32,
}

// ----- Error -----

/// ADC calibration failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalibrationError;

// ----- Driver Type -----

/// ADC driver for single-shot conversions.
///
/// Provides blocking single-ended ADC reads with configurable resolution
/// and hardware averaging. Call [`calibrate`](Adc::calibrate) after
/// construction for best accuracy.
pub struct Adc<ADC> {
    _adc: PhantomData<ADC>,
}

// ----- Extension Trait -----

/// Extension trait for creating ADC drivers from PAC ADC peripherals.
///
/// Consumes the ADC peripheral, enables its clock gate, and configures
/// sensible defaults (10-bit, bus/2 clock, /4 divider, long sample time).
pub trait AdcExt: Sized {
    type Instance;

    /// Initialize the ADC with default configuration.
    ///
    /// Defaults: 10-bit resolution, bus clock /2, ADIV /4, long sample time,
    /// software trigger, no averaging. Call [`Adc::calibrate`] afterward
    /// for best accuracy.
    fn adc(self, clocks: &Clocks, sim: &pac::Sim) -> Adc<Self::Instance>;
}

// ----- Per-instance macro -----

macro_rules! adc_impl {
    ($PacType:ty, $Instance:ty, $scgc_reg:ident, $scgc_field:ident) => {
        impl Adc<$Instance> {
            fn regs() -> &'static <$PacType as core::ops::Deref>::Target {
                unsafe { &*<$PacType>::PTR }
            }

            fn init() -> Self {
                let adc = Self::regs();

                // CFG1: bus/2 clock, 10-bit, long sample, /4 divider, normal power
                adc.cfg1().write(|w| {
                    w.adiclk()._01()    // bus clock / 2
                     .mode().bits10()   // 10-bit resolution
                     .adlsmp()._1()    // long sample time
                     .adiv()._10()     // divide by 4
                     .adlpc()._0()     // normal power
                });

                // CFG2: ADxxA channels, normal speed, longest sample
                adc.cfg2().write(|w| {
                    w.muxsel()._0()    // ADxxA channels
                     .adhsc()._0()     // normal speed
                     .adacken()._0()   // async clock disabled
                     .adlsts()._00()   // default longest sample time
                });

                // SC2: software trigger, default VREF, no compare, no DMA
                adc.sc2().write(|w| {
                    w.adtrg()._0()     // software trigger
                     .refsel().default() // VREFH/VREFL
                });

                // SC3: no averaging, no continuous, no calibration
                adc.sc3().write(|w| w);

                // SC1A: disable conversions (ADCH = 0x1F)
                adc.sc1(0).write(|w| w.adch()._11111());

                Adc { _adc: PhantomData }
            }

            /// Set the ADC conversion resolution.
            pub fn set_resolution(&mut self, res: Resolution) {
                let adc = Self::regs();
                adc.cfg1().modify(|_, w| match res {
                    Resolution::Bits8 => w.mode().bits8(),
                    Resolution::Bits10 => w.mode().bits10(),
                    Resolution::Bits12 => w.mode().bits12(),
                    Resolution::Bits16 => w.mode().bits16(),
                });
            }

            /// Set the hardware averaging mode.
            pub fn set_averaging(&mut self, avg: Averaging) {
                let adc = Self::regs();
                // Use write() instead of modify() to avoid w1c hazard with CALF
                match avg {
                    Averaging::Disabled => {
                        adc.sc3().write(|w| w);
                    }
                    Averaging::Avg4 => {
                        adc.sc3().write(|w| w.avge()._1().avgs()._00());
                    }
                    Averaging::Avg8 => {
                        adc.sc3().write(|w| w.avge()._1().avgs()._01());
                    }
                    Averaging::Avg16 => {
                        adc.sc3().write(|w| w.avge()._1().avgs()._10());
                    }
                    Averaging::Avg32 => {
                        adc.sc3().write(|w| w.avge()._1().avgs()._11());
                    }
                }
            }

            /// Run the ADC self-calibration sequence.
            ///
            /// Temporarily reconfigures the ADC for maximum accuracy (16-bit,
            /// 32-sample averaging, slow clock), then restores the previous
            /// configuration. Returns `Err` if calibration fails.
            pub fn calibrate(&mut self) -> Result<(), CalibrationError> {
                let adc = Self::regs();

                // Save current configuration
                let saved_cfg1 = adc.cfg1().read().bits();
                let saved_sc3 = adc.sc3().read().bits();

                // Configure for calibration: 16-bit, bus/2, /8, long sample
                adc.cfg1().write(|w| {
                    w.adiclk()._01()    // bus clock / 2
                     .mode().bits16()   // 16-bit for max accuracy
                     .adlsmp()._1()    // long sample time
                     .adiv()._11()     // divide by 8
                     .adlpc()._0()     // normal power
                });

                // Start calibration with 32-sample averaging
                adc.sc3().write(|w| {
                    w.cal().set_bit()
                     .avge()._1()
                     .avgs()._11()
                });

                // Wait for calibration to complete (COCO in SC1A)
                while adc.sc1(0).read().coco().is_0() {}

                // Check for calibration failure
                if adc.sc3().read().calf().is_1() {
                    // Clear CALF (w1c)
                    adc.sc3().write(|w| w.calf()._1());
                    // Restore original configuration
                    adc.cfg1().write(|w| unsafe { w.bits(saved_cfg1) });
                    adc.sc3().write(|w| unsafe { w.bits(saved_sc3 & !0x40) }); // clear CALF bit
                    return Err(CalibrationError);
                }

                // Calculate plus-side gain calibration value
                // Formula from K20 reference manual section 28.4.6
                let plus: u32 = adc.clps().read().clps().bits() as u32
                    + adc.clp4().read().clp4().bits() as u32
                    + adc.clp3().read().clp3().bits() as u32
                    + adc.clp2().read().clp2().bits() as u32
                    + adc.clp1().read().clp1().bits() as u32
                    + adc.clp0().read().clp0().bits() as u32;
                let pg = ((plus / 2) | 0x8000) as u16;
                adc.pg().write(|w| unsafe { w.pg().bits(pg) });

                // Calculate minus-side gain calibration value
                let minus: u32 = adc.clms().read().clms().bits() as u32
                    + adc.clm4().read().clm4().bits() as u32
                    + adc.clm3().read().clm3().bits() as u32
                    + adc.clm2().read().clm2().bits() as u32
                    + adc.clm1().read().clm1().bits() as u32
                    + adc.clm0().read().clm0().bits() as u32;
                let mg = ((minus / 2) | 0x8000) as u16;
                adc.mg().write(|w| unsafe { w.mg().bits(mg) });

                // Restore original configuration
                adc.cfg1().write(|w| unsafe { w.bits(saved_cfg1) });
                // Restore SC3 without CALF or CAL bits
                adc.sc3().write(|w| unsafe { w.bits(saved_sc3 & !0xC0) });

                Ok(())
            }

            /// Perform a single-shot ADC conversion on the given channel.
            ///
            /// Blocks until the conversion is complete. Channel numbers
            /// are hardware-specific (0-23 for external pins, 26=temp sensor,
            /// 27=bandgap, 29=VREFSH, 30=VREFSL).
            pub fn read(&mut self, channel: u8) -> u16 {
                let adc = Self::regs();

                // Start conversion: write channel to SC1A
                // write() starts from zero → DIFF=0 (single-ended), AIEN=0 (no interrupt)
                adc.sc1(0).write(|w| unsafe { w.adch().bits(channel & 0x1F) });

                // Wait for conversion complete
                while adc.sc1(0).read().coco().is_0() {}

                // Read result
                adc.r(0).read().d().bits()
            }
        }

        impl AdcExt for $PacType {
            type Instance = $Instance;

            fn adc(self, _clocks: &Clocks, sim: &pac::Sim) -> Adc<$Instance> {
                sim.$scgc_reg().modify(|_, w| w.$scgc_field()._1());
                Adc::<$Instance>::init()
            }
        }
    };
}

// Both variants have ADC0
adc_impl!(pac::Adc0, Adc0, scgc6, adc0);

// Only mk20d7 has ADC1
#[cfg(feature = "mk20d7")]
adc_impl!(pac::Adc1, Adc1, scgc3, adc1);
