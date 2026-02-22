use core::marker::PhantomData;

use crate::clocks::Clocks;
use crate::gpio::{Spi0SckPin, Spi0MosiPin, Spi0MisoPin};
#[cfg(feature = "mk20d7")]
use crate::gpio::{Spi1SckPin, Spi1MosiPin, Spi1MisoPin};
use crate::pac;
use crate::time::Hertz;

use embedded_hal::spi::{Phase, Polarity};

pub use embedded_hal::spi::Mode;
pub use embedded_hal::spi::{MODE_0, MODE_1, MODE_2, MODE_3};

// ----- Configuration -----

/// SPI bus configuration.
pub struct Config {
    pub baudrate: Hertz,
    pub mode: Mode,
}

impl Config {
    /// Create a new configuration with the given baud rate, defaulting to MODE_0.
    pub fn new(baudrate: Hertz) -> Self {
        Config {
            baudrate,
            mode: MODE_0,
        }
    }

    /// Set the SPI mode (polarity and phase).
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }
}

// ----- Error -----

/// SPI communication error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    Overrun,
}

impl embedded_hal::spi::Error for Error {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        embedded_hal::spi::ErrorKind::Overrun
    }
}

// ----- Instance Markers -----

mod sealed {
    pub trait SpiInstance {}
}

/// Marker type for SPI0.
pub struct Spi0;

impl sealed::SpiInstance for Spi0 {}

/// Marker type for SPI1 (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct Spi1;

#[cfg(feature = "mk20d7")]
impl sealed::SpiInstance for Spi1 {}

// ----- Driver Type -----

/// SPI master driver.
pub struct Spi<SPI> {
    _spi: PhantomData<SPI>,
}

// ----- Baud Rate Calculation -----

const BR_SCALERS: [u16; 16] = [
    2, 4, 6, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768,
];
const PBR_PRESCALERS: [u16; 4] = [2, 3, 5, 7];

/// Calculate BR scaler index, PBR prescaler index, and DBR flag for target baud rate.
///
/// Returns the combination that produces the closest baud rate not exceeding the target.
fn calc_baud(bus_clk: u32, target: u32) -> (u8, u8, bool) {
    let mut best_br: u8 = 0;
    let mut best_pbr: u8 = 0;
    let mut best_dbr = false;
    let mut best_baud: u32 = 0;

    for (pbr_idx, &pbr) in PBR_PRESCALERS.iter().enumerate() {
        for (br_idx, &br) in BR_SCALERS.iter().enumerate() {
            for dbr in [false, true] {
                let mult: u64 = if dbr { 2 } else { 1 };
                let baud = (bus_clk as u64 * mult / (pbr as u64 * br as u64)) as u32;
                if baud <= target && baud > best_baud {
                    best_baud = baud;
                    best_br = br_idx as u8;
                    best_pbr = pbr_idx as u8;
                    best_dbr = dbr;
                }
            }
        }
    }

    (best_br, best_pbr, best_dbr)
}

// ----- Extension Trait -----

/// Extension trait for creating SPI drivers from PAC SPI peripherals.
///
/// Pin types are constrained by marker traits (e.g., [`Spi0SckPin`]) to
/// ensure only valid pin assignments compile.
pub trait SpiExt<SCK, MOSI, MISO>: Sized {
    type Instance: sealed::SpiInstance;

    fn spi(
        self,
        _sck: SCK,
        _mosi: MOSI,
        _miso: MISO,
        config: Config,
        clocks: &Clocks,
        sim: &pac::Sim,
    ) -> Spi<Self::Instance>;
}

// ----- Per-instance macro -----

macro_rules! spi_impl {
    ($PacType:ty, $Instance:ty, $ctar_fn:ident, $pushr_fn:ident, $scgc_field:ident,
     $SckPin:path, $MosiPin:path, $MisoPin:path,
     $tx_source:expr, $rx_source:expr) => {
        impl Spi<$Instance> {
            fn regs() -> &'static <$PacType as core::ops::Deref>::Target {
                unsafe { &*<$PacType>::PTR }
            }

            fn init(config: Config, bus_clk: u32) -> Self {
                let spi = Self::regs();
                let (br, pbr, dbr) = calc_baud(bus_clk, config.baudrate.raw());

                // 1. Halt + Master mode + Enable module clocks, PCS0 inactive high
                spi.mcr().write(|w| {
                    w.mstr()._1()
                     .halt()._1()
                     .mdis()._0()
                     .pcsis()._1()
                });

                // 2. Flush FIFOs (CLR_TXF/CLR_RXF are self-clearing)
                spi.mcr().modify(|_, w| w.clr_txf()._1().clr_rxf()._1());

                // 3. Clear all status flags (w1c — use write, not modify)
                spi.sr().write(|w| {
                    w.tcf()._1()
                     .eoqf()._1()
                     .tfuf()._1()
                     .rfof()._1()
                     .tfff()._1()
                     .rfdf()._1()
                });

                // 4. Configure CTAR0: 8-bit frame, MSB first, computed baud rate
                spi.$ctar_fn(0).write(|w| {
                    let w = match config.mode.polarity {
                        Polarity::IdleHigh => w.cpol()._1(),
                        Polarity::IdleLow => w.cpol()._0(),
                    };
                    let w = match config.mode.phase {
                        Phase::CaptureOnSecondTransition => w.cpha()._1(),
                        Phase::CaptureOnFirstTransition => w.cpha()._0(),
                    };
                    let w = match pbr {
                        0 => w.pbr()._00(),
                        1 => w.pbr()._01(),
                        2 => w.pbr()._10(),
                        _ => w.pbr()._11(),
                    };
                    let w = if dbr { w.dbr()._1() } else { w.dbr()._0() };
                    let w = w.lsbfe()._0()
                              .pcssck()._00()
                              .pasc()._00()
                              .pdt()._00();
                    unsafe {
                        w.fmsz().bits(7) // 8-bit frame (N-1)
                         .br().bits(br)
                         .cssck().bits(0)
                         .asc().bits(0)
                         .dt().bits(0)
                    }
                });

                // 5. Start transfers
                spi.mcr().modify(|_, w| w.halt()._0());

                Spi { _spi: PhantomData }
            }

            fn transfer_byte(byte: u8) -> Result<u8, Error> {
                let spi = Self::regs();

                // Wait for TX FIFO space
                while spi.sr().read().tfff().is_0() {}
                // Clear TFFF (w1c)
                spi.sr().write(|w| w.tfff()._1());

                // Push data with PCS0 asserted, CTAR0
                spi.$pushr_fn().write(|w| unsafe {
                    w.txdata().bits(byte as u16)
                     .pcs()._1()
                });

                // Wait for RX FIFO data
                while spi.sr().read().rfdf().is_0() {}

                // Check for RX overflow
                if spi.sr().read().rfof().is_1() {
                    spi.sr().write(|w| w.rfof()._1());
                    spi.sr().write(|w| w.rfdf()._1());
                    let _ = spi.popr().read();
                    return Err(Error::Overrun);
                }

                // Clear RFDF (w1c)
                spi.sr().write(|w| w.rfdf()._1());

                // Pop received byte
                Ok(spi.popr().read().rxdata().bits() as u8)
            }

            /// Return the TX (PUSHR) register address for DMA configuration.
            ///
            /// Use with [`DmaChannel::configure_peripheral_write`] and
            /// the appropriate `DmaSource` (e.g., `DmaSource::SPI0_TX`).
            pub fn tx_dma_addr() -> u32 {
                <$PacType>::PTR as u32 + 0x34 // PUSHR offset
            }

            /// Return the RX (POPR) register address for DMA configuration.
            ///
            /// Use with [`DmaChannel::configure_peripheral_read`] and
            /// the appropriate `DmaSource` (e.g., `DmaSource::SPI0_RX`).
            pub fn rx_dma_addr() -> u32 {
                <$PacType>::PTR as u32 + 0x38 // POPR offset
            }

            /// Start a DMA-backed write transfer.
            ///
            /// Configures the DMA channel to write `buf.len()` bytes from `buf`
            /// to the SPI TX register, then enables hardware DMA requests.
            /// The SPI TFFF_RE bit is set to trigger DMA when the TX FIFO has space.
            ///
            /// Returns a [`DmaTransfer`] handle. The transfer runs in the background;
            /// call [`DmaTransfer::wait`] to block until complete.
            ///
            /// # Safety
            ///
            /// The caller must ensure `buf` remains valid for the duration of the
            /// transfer (enforced by the lifetime on `DmaTransfer`).
            pub fn write_dma<'a, const CH: u8>(
                &'a mut self,
                buf: &'a [u8],
                ch: &'a mut crate::dma::DmaChannel<CH>,
            ) -> crate::dma::DmaTransfer<'a, CH> {
                let spi = Self::regs();

                // Configure DMA channel: memory → SPI PUSHR
                unsafe {
                    ch.configure_peripheral_write(
                        buf.as_ptr(),
                        Self::tx_dma_addr(),
                        crate::dma::TransferSize::Bits8,
                        buf.len() as u16,
                    );
                }

                // Set DMAMUX source for SPI TX
                ch.set_source($tx_source);

                // Enable SPI DMA TX request (RSER.TFFF_RE)
                spi.rser().modify(|_, w| w.tfff_re()._1().tfff_dirs()._1());

                // Enable DMA requests
                ch.enable_request();

                crate::dma::DmaTransfer { channel: ch }
            }

            /// Release the SPI peripheral, returning the PAC type.
            ///
            /// Halts transfers and disables the module before releasing.
            /// Pins are not returned since they were consumed during construction.
            ///
            /// # Safety
            ///
            /// The caller must ensure no other code holds a reference to this
            /// peripheral's registers.
            pub unsafe fn release(self) -> $PacType {
                let spi = Self::regs();
                // Halt transfers and disable module
                spi.mcr().modify(|_, w| w.halt()._1().mdis()._1());
                <$PacType>::steal()
            }
        }

        impl embedded_hal::spi::ErrorType for Spi<$Instance> {
            type Error = Error;
        }

        impl embedded_hal::spi::SpiBus<u8> for Spi<$Instance> {
            fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
                for slot in buf.iter_mut() {
                    *slot = Spi::<$Instance>::transfer_byte(0x00)?;
                }
                Ok(())
            }

            fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
                for &byte in buf {
                    let _ = Spi::<$Instance>::transfer_byte(byte)?;
                }
                Ok(())
            }

            fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
                let len = read.len().max(write.len());
                for i in 0..len {
                    let tx = write.get(i).copied().unwrap_or(0x00);
                    let rx = Spi::<$Instance>::transfer_byte(tx)?;
                    if let Some(slot) = read.get_mut(i) {
                        *slot = rx;
                    }
                }
                Ok(())
            }

            fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
                for slot in buf.iter_mut() {
                    *slot = Spi::<$Instance>::transfer_byte(*slot)?;
                }
                Ok(())
            }

            fn flush(&mut self) -> Result<(), Self::Error> {
                let spi = Spi::<$Instance>::regs();
                while spi.sr().read().txctr().bits() != 0 {}
                Ok(())
            }
        }

        impl<SCK: $SckPin, MOSI: $MosiPin, MISO: $MisoPin> SpiExt<SCK, MOSI, MISO> for $PacType {
            type Instance = $Instance;

            fn spi(
                self,
                _sck: SCK,
                _mosi: MOSI,
                _miso: MISO,
                config: Config,
                clocks: &Clocks,
                sim: &pac::Sim,
            ) -> Spi<$Instance> {
                sim.scgc6().modify(|_, w| w.$scgc_field()._1());
                Spi::<$Instance>::init(config, clocks.bus_clk().raw())
            }
        }
    };
}

// Both variants have SPI0
spi_impl!(pac::Spi0, Spi0, spi0_ctar, spi0_pushr, spi0,
          Spi0SckPin, Spi0MosiPin, Spi0MisoPin,
          crate::dma::DmaSource::SPI0_TX, crate::dma::DmaSource::SPI0_RX);

// Only mk20d7 has SPI1
#[cfg(feature = "mk20d7")]
spi_impl!(pac::Spi1, Spi1, spi1_ctar, spi1_pushr, spi1,
          Spi1SckPin, Spi1MosiPin, Spi1MisoPin,
          crate::dma::DmaSource::SPI1_TX, crate::dma::DmaSource::SPI1_RX);

// ----- Async support -----

#[cfg(feature = "async")]
mod async_impl {
    use super::*;
    use embassy_sync::waitqueue::AtomicWaker;

    static SPI0_WAKER: AtomicWaker = AtomicWaker::new();
    #[cfg(feature = "mk20d7")]
    static SPI1_WAKER: AtomicWaker = AtomicWaker::new();

    fn on_spi_interrupt(
        spi: &pac::spi0::RegisterBlock,
        waker: &AtomicWaker,
    ) {
        let sr = spi.sr().read();
        if sr.tcf().is_1() {
            // Clear TCF (w1c)
            spi.sr().write(|w| w.tcf()._1());
            // Disable TCF interrupt
            spi.rser().modify(|_, w| w.tcf_re()._0());
            waker.wake();
        }
    }

    /// Call from the `SPI0` interrupt handler.
    pub fn on_spi0_interrupt() {
        on_spi_interrupt(unsafe { &*pac::Spi0::PTR }, &SPI0_WAKER);
    }

    /// Call from the `SPI1` interrupt handler (mk20d7 only).
    #[cfg(feature = "mk20d7")]
    pub fn on_spi1_interrupt() {
        on_spi_interrupt(
            unsafe { &*(pac::Spi1::PTR as *const pac::spi0::RegisterBlock) },
            &SPI1_WAKER,
        );
    }

    macro_rules! spi_async_impl {
        ($Instance:ty, $PacType:ty, $pushr_fn:ident, $waker:expr) => {
            impl Spi<$Instance> {
                async fn transfer_byte_async(byte: u8) -> Result<u8, Error> {
                    let spi = Self::regs();

                    // Wait for TX FIFO space
                    while spi.sr().read().tfff().is_0() {}
                    spi.sr().write(|w| w.tfff()._1());

                    // Push data with PCS0 asserted, CTAR0
                    spi.$pushr_fn().write(|w| unsafe {
                        w.txdata().bits(byte as u16).pcs()._1()
                    });

                    // Enable TCF interrupt and await transfer complete
                    spi.rser().modify(|_, w| w.tcf_re()._1());
                    core::future::poll_fn(|cx| {
                        $waker.register(cx.waker());
                        // Check if TCF_RE is disabled — means ISR already fired
                        if spi.rser().read().tcf_re().is_0() {
                            core::task::Poll::Ready(())
                        } else if spi.sr().read().tcf().is_1() {
                            // Race: TCF set but ISR hasn't run yet
                            spi.sr().write(|w| w.tcf()._1());
                            spi.rser().modify(|_, w| w.tcf_re()._0());
                            core::task::Poll::Ready(())
                        } else {
                            core::task::Poll::Pending
                        }
                    })
                    .await;

                    // Check for RX overflow
                    if spi.sr().read().rfof().is_1() {
                        spi.sr().write(|w| w.rfof()._1());
                        spi.sr().write(|w| w.rfdf()._1());
                        let _ = spi.popr().read();
                        return Err(Error::Overrun);
                    }

                    spi.sr().write(|w| w.rfdf()._1());
                    Ok(spi.popr().read().rxdata().bits() as u8)
                }
            }

            #[cfg(feature = "async")]
            impl embedded_hal_async::spi::SpiBus<u8> for Spi<$Instance> {
                async fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
                    for slot in buf.iter_mut() {
                        *slot = Spi::<$Instance>::transfer_byte_async(0x00).await?;
                    }
                    Ok(())
                }

                async fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
                    for &byte in buf {
                        let _ = Spi::<$Instance>::transfer_byte_async(byte).await?;
                    }
                    Ok(())
                }

                async fn transfer(
                    &mut self,
                    read: &mut [u8],
                    write: &[u8],
                ) -> Result<(), Self::Error> {
                    let len = read.len().max(write.len());
                    for i in 0..len {
                        let tx = write.get(i).copied().unwrap_or(0x00);
                        let rx = Spi::<$Instance>::transfer_byte_async(tx).await?;
                        if let Some(slot) = read.get_mut(i) {
                            *slot = rx;
                        }
                    }
                    Ok(())
                }

                async fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
                    for slot in buf.iter_mut() {
                        *slot = Spi::<$Instance>::transfer_byte_async(*slot).await?;
                    }
                    Ok(())
                }

                async fn flush(&mut self) -> Result<(), Self::Error> {
                    let spi = Spi::<$Instance>::regs();
                    while spi.sr().read().txctr().bits() != 0 {}
                    Ok(())
                }
            }
        };
    }

    spi_async_impl!(Spi0, pac::Spi0, spi0_pushr, SPI0_WAKER);
    #[cfg(feature = "mk20d7")]
    spi_async_impl!(Spi1, pac::Spi1, spi1_pushr, SPI1_WAKER);
}

#[cfg(feature = "async")]
pub use async_impl::on_spi0_interrupt;
#[cfg(all(feature = "async", feature = "mk20d7"))]
pub use async_impl::on_spi1_interrupt;
