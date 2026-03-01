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
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    ///
    /// The `clocks` parameter is required to prove that clocks have been
    /// configured, ensuring the ADC input clock is valid. The ADC clock
    /// is derived from the bus clock via the ADICLK and ADIV settings.
    fn adc(self, clocks: &Clocks, sim: &pac::Sim) -> Adc<Self::Instance>;
}

// ----- Per-instance macro -----

macro_rules! adc_impl {
    ($PacType:ty, $Instance:ty, $scgc_reg:ident, $scgc_field:ident, $dma_source:expr) => {
        impl Adc<$Instance> {
            fn regs() -> &'static <$PacType as core::ops::Deref>::Target {
                // SAFETY: PTR is a valid pointer to the ADC register block.
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
            ///
            /// Blocks until calibration completes. This assumes the ADC
            /// hardware is functioning correctly — there is no timeout.
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
                    // SAFETY: Restoring previously-read register value.
                    adc.cfg1().write(|w| unsafe { w.bits(saved_cfg1) });
                    // SAFETY: Restoring saved_sc3 with CALF bit (bit 6) cleared.
                    // Using raw bits because modify() would risk a w1c hazard on CALF.
                    adc.sc3().write(|w| unsafe { w.bits(saved_sc3 & !(1 << 6)) });
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
                // SAFETY: pg is a 16-bit field; the calibration formula produces
                // a value with bit 15 set (the 0x8000 OR), fitting in u16.
                adc.pg().write(|w| unsafe { w.pg().bits(pg) });

                // Calculate minus-side gain calibration value
                let minus: u32 = adc.clms().read().clms().bits() as u32
                    + adc.clm4().read().clm4().bits() as u32
                    + adc.clm3().read().clm3().bits() as u32
                    + adc.clm2().read().clm2().bits() as u32
                    + adc.clm1().read().clm1().bits() as u32
                    + adc.clm0().read().clm0().bits() as u32;
                let mg = ((minus / 2) | 0x8000) as u16;
                // SAFETY: mg is a 16-bit field; same calibration formula as pg.
                adc.mg().write(|w| unsafe { w.mg().bits(mg) });

                // SAFETY: Restoring previously-read register value.
                adc.cfg1().write(|w| unsafe { w.bits(saved_cfg1) });
                // Restore SC3 without CALF or CAL bits
                // SAFETY: Restoring saved_sc3 with CAL (bit 7) and CALF (bit 6) cleared.
                adc.sc3().write(|w| unsafe { w.bits(saved_sc3 & !((1 << 7) | (1 << 6))) });

                Ok(())
            }

            /// Perform a single-shot ADC conversion on the given channel.
            ///
            /// Blocks until the conversion is complete. This assumes the ADC
            /// hardware is functioning correctly — there is no timeout.
            ///
            /// Channel numbers are hardware-specific (0-23 for external pins,
            /// 26=temp sensor, 27=bandgap, 29=VREFSH, 30=VREFSL).
            pub fn read(&mut self, channel: u8) -> u16 {
                let adc = Self::regs();

                // Start conversion: write channel to SC1A
                // write() starts from zero → DIFF=0 (single-ended), AIEN=0 (no interrupt)
                // SAFETY: adch is a 5-bit field; masked to 0x1F.
                adc.sc1(0).write(|w| unsafe { w.adch().bits(channel & 0x1F) });

                // Wait for conversion complete
                while adc.sc1(0).read().coco().is_0() {}

                // Read result
                adc.r(0).read().d().bits()
            }

            /// Release the ADC peripheral, returning the PAC type.
            ///
            /// Pins are not returned since they were consumed during construction.
            ///
            /// # Safety
            ///
            /// The caller must ensure no other code holds a reference to this
            /// peripheral's registers.
            pub unsafe fn release(self) -> $PacType {
                <$PacType>::steal()
            }

            /// Return the result register (RA) address for DMA configuration.
            pub fn result_dma_addr() -> u32 {
                <$PacType>::PTR as u32 + 0x10 // R[0] (RA) offset
            }

            /// Start a DMA-backed multi-sample read.
            ///
            /// Configures the ADC for continuous conversion with DMA enabled.
            /// Each conversion result is written to `results` via DMA.
            /// The DMA major loop count equals `results.len()`.
            ///
            /// After the transfer completes, continuous mode and DMA are disabled.
            ///
            /// # Arguments
            /// * `channel` — ADC input channel (0-23, or special channels).
            /// * `results` — Buffer for conversion results (16-bit each).
            /// * `ch` — DMA channel to use.
            pub fn read_dma<'a, const DMA_CH: u8>(
                &'a mut self,
                channel: u8,
                results: &'a mut [u16],
                ch: &'a mut crate::dma::DmaChannel<DMA_CH>,
            ) -> crate::dma::DmaTransfer<'a, DMA_CH> {
                let adc = Self::regs();

                // Enable DMA and continuous conversion
                adc.sc2().modify(|_, w| w.dmaen()._1());
                adc.sc3().write(|w| w.adco()._1());

                // Configure DMA: ADC RA → memory buffer (16-bit)
                unsafe {
                    ch.configure_peripheral_read(
                        Self::result_dma_addr(),
                        results.as_mut_ptr() as *mut u8,
                        crate::dma::TransferSize::Bits16,
                        results.len() as u16,
                    );
                }

                ch.set_source($dma_source);
                ch.enable_request();

                // Start first conversion
                // SAFETY: adch is a 5-bit field; masked to 0x1F.
                adc.sc1(0).write(|w| unsafe { w.adch().bits(channel & 0x1F) });

                crate::dma::DmaTransfer { channel: ch }
            }
        }

        impl AdcExt for $PacType {
            type Instance = $Instance;

            fn adc(self, _clocks: &Clocks, sim: &pac::Sim) -> Adc<$Instance> {
                sim.$scgc_reg().modify(|_, w| w.$scgc_field().enabled());
                Adc::<$Instance>::init()
            }
        }
    };
}

// Both variants have ADC0
adc_impl!(pac::Adc0, Adc0, scgc6, adc0, crate::dma::DmaSource::ADC0);

// Only mk20d7 has ADC1
#[cfg(feature = "mk20d7")]
adc_impl!(pac::Adc1, Adc1, scgc3, adc1, crate::dma::DmaSource::ADC1);

// ----- PDB-triggered continuous scanning -----

/// Configuration for PDB-triggered continuous ADC scanning.
pub struct ScanConfig {
    /// ADC channels to scan (1-2 for basic mode).
    ///
    /// For 3+ channels, DMA channel linking is used to cycle the ADC
    /// channel mux automatically.
    pub channels: &'static [u8],
    /// PDB counter modulus (sets scan repetition period in PDB clock ticks).
    pub modulus: u16,
    /// PDB prescaler divider.
    pub prescaler: crate::pdb::Prescaler,
    /// PDB prescaler multiplication factor.
    pub multiplier: crate::pdb::Multiplier,
}

/// Handle for a running continuous ADC scan.
///
/// Dropping this value stops the scan by disabling PDB, ADC hardware
/// trigger, and DMA.
pub struct ContinuousScan<'a, const DMA_CH: u8> {
    dma_ch: &'a mut crate::dma::DmaChannel<DMA_CH>,
    pdb: &'a mut crate::pdb::Pdb,
    results: *const u16,
    num_channels: usize,
}

impl<'a, const DMA_CH: u8> ContinuousScan<'a, DMA_CH> {
    /// Read the latest value for a channel index.
    ///
    /// Uses a volatile read since DMA writes asynchronously.
    ///
    /// # Panics
    ///
    /// Panics if `index >= num_channels`.
    pub fn read_latest(&self, index: usize) -> u16 {
        assert!(index < self.num_channels);
        // SAFETY: DMA is writing to results buffer asynchronously. Volatile
        // read ensures we get the latest value without compiler reordering.
        unsafe { core::ptr::read_volatile(self.results.add(index)) }
    }

    /// Stop the scan and return borrowed resources.
    pub fn stop(mut self) -> (&'a mut crate::dma::DmaChannel<DMA_CH>, &'a mut crate::pdb::Pdb) {
        self.cleanup();
        let dma_ch_ptr = self.dma_ch as *mut crate::dma::DmaChannel<DMA_CH>;
        let pdb_ptr = self.pdb as *mut crate::pdb::Pdb;
        // SAFETY: We need to return the references while preventing Drop.
        // The raw pointers preserve the original 'a lifetime, and cleanup()
        // ensures hardware is stopped before we return the references.
        core::mem::forget(self);
        unsafe { (&mut *dma_ch_ptr, &mut *pdb_ptr) }
    }

    fn cleanup(&mut self) {
        // Disable PDB
        self.pdb.disable();
        // Disable DMA request
        self.dma_ch.disable_request();
        self.dma_ch.clear_done();
    }
}

impl<'a, const DMA_CH: u8> Drop for ContinuousScan<'a, DMA_CH> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

macro_rules! adc_scan_impl {
    ($PacType:ty, $Instance:ty, $dma_source:expr, $pdb_channel:literal) => {
        impl Adc<$Instance> {
            /// Start a PDB-triggered continuous multi-channel ADC scan.
            ///
            /// Configures the ADC for hardware-triggered conversions, sets up
            /// PDB pre-triggers for each channel, and uses DMA to read results
            /// into the provided buffer.
            ///
            /// # 2-channel path
            ///
            /// For 1-2 channels, PDB pre-trigger 0 triggers SC1A and
            /// pre-trigger 1 (back-to-back) triggers SC1B. Each conversion
            /// result is DMA'd from the result register to the buffer.
            ///
            /// # Arguments
            ///
            /// * `config` — Scan configuration (channels, timing).
            /// * `results` — Buffer for conversion results. Must have
            ///   `config.channels.len()` elements.
            /// * `dma_ch` — DMA channel for ADC result reads.
            /// * `pdb` — PDB driver.
            ///
            /// # Panics
            ///
            /// Panics if `config.channels` is empty, has more than 2 entries,
            /// or if `results.len() < config.channels.len()`.
            pub fn start_continuous_scan<'a, const DMA_CH: u8>(
                &'a mut self,
                config: &ScanConfig,
                results: &'a mut [u16],
                dma_ch: &'a mut crate::dma::DmaChannel<DMA_CH>,
                pdb: &'a mut crate::pdb::Pdb,
            ) -> ContinuousScan<'a, DMA_CH> {
                let num_ch = config.channels.len();
                assert!(num_ch >= 1 && num_ch <= 2);
                assert!(results.len() >= num_ch);

                let adc = Self::regs();

                // 1. Enable hardware trigger mode and DMA
                adc.sc2().modify(|_, w| w.adtrg()._1().dmaen()._1());
                // Disable continuous mode — PDB triggers each conversion
                adc.sc3().write(|w| w);

                // 2. Write channel mux values to SC1A (and SC1B if 2 channels)
                // SAFETY: adch is a 5-bit field; masked to 0x1F.
                adc.sc1(0).write(|w| unsafe {
                    w.adch().bits(config.channels[0] & 0x1F)
                });
                if num_ch >= 2 {
                    adc.sc1(1).write(|w| unsafe {
                        w.adch().bits(config.channels[1] & 0x1F)
                    });
                }

                // 3. Configure PDB
                pdb.configure(
                    crate::pdb::TriggerSource::Software,
                    config.prescaler,
                    config.multiplier,
                    config.modulus,
                );
                pdb.set_continuous(true);

                // Pre-trigger 0: delay=0, triggers SC1A → channel[0]
                pdb.set_pretrigger_delay($pdb_channel, 0, 0);
                pdb.enable_pretrigger($pdb_channel, 0);

                if num_ch >= 2 {
                    // Pre-trigger 1: back-to-back after pre-trigger 0
                    // triggers SC1B → channel[1]
                    pdb.enable_pretrigger($pdb_channel, 1);
                    pdb.enable_back_to_back($pdb_channel, 1);
                }

                pdb.load_ok();

                // 4. Configure DMA: peripheral read from ADC RA → results buffer
                let total_bytes = num_ch as i32 * 2; // 16-bit per result
                unsafe {
                    dma_ch.configure(&crate::dma::TransferConfig {
                        source_addr: Self::result_dma_addr(),
                        dest_addr: results.as_mut_ptr() as u32,
                        source_size: crate::dma::TransferSize::Bits16,
                        dest_size: crate::dma::TransferSize::Bits16,
                        source_offset: 0,     // Fixed peripheral address
                        dest_offset: 2,       // Advance 2 bytes per result
                        minor_loop_bytes: 2,  // 1 result per DMA activation
                        major_loop_count: num_ch as u16,
                        source_last_adjust: 0,
                        dest_last_adjust: -total_bytes, // Reset buffer pointer
                        dest_modulo: 0,
                        auto_disable: true,
                    });
                }

                dma_ch.set_source($dma_source);
                dma_ch.enable_request();

                // 5. Enable PDB and trigger
                pdb.enable();
                pdb.software_trigger();

                ContinuousScan {
                    dma_ch,
                    pdb,
                    results: results.as_ptr(),
                    num_channels: num_ch,
                }
            }
        }
    };
}

adc_scan_impl!(pac::Adc0, Adc0, crate::dma::DmaSource::ADC0, 0);
#[cfg(feature = "mk20d7")]
adc_scan_impl!(pac::Adc1, Adc1, crate::dma::DmaSource::ADC1, 1);
