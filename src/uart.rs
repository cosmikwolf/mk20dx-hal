use core::marker::PhantomData;

use crate::clocks::Clocks;
use crate::gpio::{Uart0TxPin, Uart0RxPin, Uart1TxPin, Uart1RxPin, Uart2TxPin, Uart2RxPin};
use crate::pac;
use crate::time::Hertz;

// ----- Configuration -----

/// UART serial port configuration.
pub struct Config {
    pub baudrate: Hertz,
    pub parity: Parity,
    pub word_length: WordLength,
}

impl Config {
    /// Create a new configuration with the given baud rate and 8-N-1 defaults.
    pub fn new(baudrate: Hertz) -> Self {
        Config {
            baudrate,
            parity: Parity::None,
            word_length: WordLength::Bits8,
        }
    }

    /// Set parity mode.
    pub fn parity(mut self, parity: Parity) -> Self {
        self.parity = parity;
        self
    }

    /// Set word length.
    pub fn word_length(mut self, word_length: WordLength) -> Self {
        self.word_length = word_length;
        self
    }
}

/// Parity setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// Word length setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordLength {
    Bits8,
    Bits9,
}

// ----- Error -----

/// UART communication error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    Overrun,
    Framing,
    Parity,
    Noise,
}

impl embedded_hal_nb::serial::Error for Error {
    fn kind(&self) -> embedded_hal_nb::serial::ErrorKind {
        match self {
            Error::Overrun => embedded_hal_nb::serial::ErrorKind::Overrun,
            Error::Framing => embedded_hal_nb::serial::ErrorKind::FrameFormat,
            Error::Parity => embedded_hal_nb::serial::ErrorKind::Parity,
            Error::Noise => embedded_hal_nb::serial::ErrorKind::Noise,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Overrun => write!(f, "overrun"),
            Error::Framing => write!(f, "framing error"),
            Error::Parity => write!(f, "parity error"),
            Error::Noise => write!(f, "noise detected"),
        }
    }
}

impl core::error::Error for Error {}

impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

// ----- Instance Abstraction -----

mod sealed {
    pub trait UartInstance {
        type Pac;
        fn ptr() -> *const crate::pac::uart0::RegisterBlock;
        fn enable_clock(sim: &crate::pac::Sim);
        /// Reconstruct the PAC peripheral (unsound if aliased).
        unsafe fn steal_pac() -> Self::Pac;
        /// DMAMUX source for UART TX.
        fn dma_source_tx() -> crate::dma::DmaSource;
        /// DMAMUX source for UART RX.
        fn dma_source_rx() -> crate::dma::DmaSource;
    }
}

/// Marker type for UART0.
pub struct Uart0;
/// Marker type for UART1.
pub struct Uart1;
/// Marker type for UART2.
pub struct Uart2;

impl sealed::UartInstance for Uart0 {
    type Pac = pac::Uart0;
    fn ptr() -> *const pac::uart0::RegisterBlock {
        pac::Uart0::PTR
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc4().modify(|_, w| w.uart0()._1());
    }
    unsafe fn steal_pac() -> pac::Uart0 {
        pac::Uart0::steal()
    }
    fn dma_source_tx() -> crate::dma::DmaSource {
        crate::dma::DmaSource::UART0_TX
    }
    fn dma_source_rx() -> crate::dma::DmaSource {
        crate::dma::DmaSource::UART0_RX
    }
}

impl sealed::UartInstance for Uart1 {
    type Pac = pac::Uart1;
    fn ptr() -> *const pac::uart0::RegisterBlock {
        // Safety: UART1 and UART0 have identical register layouts through offset
        // 0x16 (rcfifo). The HAL only accesses registers within this range.
        pac::Uart1::PTR as *const pac::uart0::RegisterBlock
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc4().modify(|_, w| w.uart1()._1());
    }
    unsafe fn steal_pac() -> pac::Uart1 {
        pac::Uart1::steal()
    }
    fn dma_source_tx() -> crate::dma::DmaSource {
        crate::dma::DmaSource::UART1_TX
    }
    fn dma_source_rx() -> crate::dma::DmaSource {
        crate::dma::DmaSource::UART1_RX
    }
}

impl sealed::UartInstance for Uart2 {
    type Pac = pac::Uart2;
    fn ptr() -> *const pac::uart0::RegisterBlock {
        // Safety: UART2 and UART0 have identical register layouts through offset
        // 0x16 (rcfifo). The HAL only accesses registers within this range.
        pac::Uart2::PTR as *const pac::uart0::RegisterBlock
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc4().modify(|_, w| w.uart2()._1());
    }
    unsafe fn steal_pac() -> pac::Uart2 {
        pac::Uart2::steal()
    }
    fn dma_source_tx() -> crate::dma::DmaSource {
        crate::dma::DmaSource::UART2_TX
    }
    fn dma_source_rx() -> crate::dma::DmaSource {
        crate::dma::DmaSource::UART2_RX
    }
}

fn regs<UART: sealed::UartInstance>() -> &'static pac::uart0::RegisterBlock {
    unsafe { &*UART::ptr() }
}

// ----- Driver Types -----

/// UART serial driver (combined TX + RX).
pub struct Serial<UART> {
    _uart: PhantomData<UART>,
}

/// UART transmit half.
pub struct Tx<UART> {
    _uart: PhantomData<UART>,
}

/// UART receive half.
pub struct Rx<UART> {
    _uart: PhantomData<UART>,
}

// ----- Baud Rate Calculation -----

/// Calculate SBR and BRFA for the target baud rate.
///
/// `baud = module_clk / (16 * (SBR + BRFA/32))`
fn calc_baud(module_clk: u32, baudrate: u32) -> (u16, u8) {
    // sbr32 = module_clk * 2 / baudrate (with rounding)
    let sbr32 = ((2 * module_clk as u64 + baudrate as u64 / 2) / baudrate as u64) as u32;
    let sbr = (sbr32 / 32).clamp(1, 0x1FFF) as u16;
    let brfa = (sbr32 % 32) as u8;
    (sbr, brfa)
}

// ----- Initialization -----

impl<UART: sealed::UartInstance> Serial<UART> {
    fn init(config: Config, module_clk: u32) -> Self {
        let uart = regs::<UART>();
        let (sbr, brfa) = calc_baud(module_clk, config.baudrate.raw());

        // Disable TX and RX during configuration
        uart.c2().write(|w| unsafe { w.bits(0) });

        // Baud rate fine adjust
        uart.c4().write(|w| unsafe { w.brfa().bits(brfa) });

        // Word length and parity
        uart.c1().write(|w| {
            let w = match config.word_length {
                WordLength::Bits8 if config.parity != Parity::None => w.m()._1(),
                WordLength::Bits8 => w.m()._0(),
                WordLength::Bits9 => w.m()._1(),
            };
            match config.parity {
                Parity::None => w.pe()._0(),
                Parity::Even => w.pe()._1().pt()._0(),
                Parity::Odd => w.pe()._1().pt()._1(),
            }
        });

        // Baud rate divisor (BDL write latches the pair)
        uart.bdh().write(|w| unsafe { w.sbr().bits((sbr >> 8) as u8) });
        uart.bdl().write(|w| unsafe { w.sbr().bits(sbr as u8) });

        // Enable FIFO (harmless on depth-1 UARTs)
        uart.pfifo().modify(|_, w| w.txfe()._1().rxfe()._1());

        // Flush FIFOs
        uart.cfifo().write(|w| w.txflush()._1().rxflush()._1());

        // TX watermark = 0, RX watermark = 1
        uart.twfifo().write(|w| unsafe { w.bits(0) });
        uart.rwfifo().write(|w| unsafe { w.bits(1) });

        // Enable TX and RX
        uart.c2().write(|w| w.te()._1().re()._1());

        Serial { _uart: PhantomData }
    }

    /// Split into independent TX and RX halves.
    pub fn split(self) -> (Tx<UART>, Rx<UART>) {
        (Tx { _uart: PhantomData }, Rx { _uart: PhantomData })
    }

    /// Recombine TX and RX halves.
    pub fn join(_tx: Tx<UART>, _rx: Rx<UART>) -> Self {
        Serial { _uart: PhantomData }
    }

    /// Return the data register address for DMA configuration.
    pub fn data_dma_addr() -> u32 {
        UART::ptr() as u32 + 0x07 // D register offset
    }

    /// Start a DMA-backed write transfer.
    ///
    /// Configures the DMA channel to transmit `buf.len()` bytes via DMA.
    /// The UART's TDMAS bit enables hardware DMA TX requests.
    ///
    /// Returns a [`DmaTransfer`](crate::dma::DmaTransfer) handle. Call
    /// [`wait()`](crate::dma::DmaTransfer::wait) to block until complete.
    pub fn write_dma<'a, const CH: u8>(
        &'a mut self,
        buf: &'a [u8],
        ch: &'a mut crate::dma::DmaChannel<CH>,
    ) -> crate::dma::DmaTransfer<'a, CH> {
        let uart = regs::<UART>();

        // Configure DMA: memory → UART D register
        unsafe {
            ch.configure_peripheral_write(
                buf.as_ptr(),
                Self::data_dma_addr(),
                crate::dma::TransferSize::Bits8,
                buf.len() as u16,
            );
        }

        ch.set_source(UART::dma_source_tx());

        // Enable DMA TX requests: C5.TDMAS=1, C2.TIE=1
        uart.c5().modify(|_, w| w.tdmas()._1());
        uart.c2().modify(|_, w| w.tie()._1());

        ch.enable_request();

        crate::dma::DmaTransfer { channel: ch }
    }

    /// Start a DMA-backed read transfer.
    ///
    /// Configures the DMA channel to receive `buf.len()` bytes via DMA.
    /// The UART's RDMAS bit enables hardware DMA RX requests.
    ///
    /// Returns a [`DmaTransfer`](crate::dma::DmaTransfer) handle. Call
    /// [`wait()`](crate::dma::DmaTransfer::wait) to block until complete.
    pub fn read_dma<'a, const CH: u8>(
        &'a mut self,
        buf: &'a mut [u8],
        ch: &'a mut crate::dma::DmaChannel<CH>,
    ) -> crate::dma::DmaTransfer<'a, CH> {
        let uart = regs::<UART>();

        // Configure DMA: UART D register → memory
        unsafe {
            ch.configure_peripheral_read(
                Self::data_dma_addr(),
                buf.as_mut_ptr(),
                crate::dma::TransferSize::Bits8,
                buf.len() as u16,
            );
        }

        ch.set_source(UART::dma_source_rx());

        // Enable DMA RX requests: C5.RDMAS=1, C2.RIE=1
        uart.c5().modify(|_, w| w.rdmas()._1());
        uart.c2().modify(|_, w| w.rie()._1());

        ch.enable_request();

        crate::dma::DmaTransfer { channel: ch }
    }

    /// Release the UART peripheral, returning the PAC type.
    ///
    /// Disables the transmitter and receiver before releasing.
    /// Pins are not returned since they were consumed during construction;
    /// reconfigure them via the GPIO port after release.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code holds a reference to this
    /// peripheral's registers (e.g., via split TX/RX halves).
    pub unsafe fn release(self) -> UART::Pac {
        let uart = regs::<UART>();
        // Disable TX and RX
        uart.c2().write(|w| w.te()._0().re()._0());
        UART::steal_pac()
    }
}

// ----- Non-blocking helpers -----

fn nb_read<UART: sealed::UartInstance>() -> nb::Result<u8, Error> {
    let uart = regs::<UART>();
    let s1 = uart.s1().read();

    // Check error flags (reading S1 then D clears them)
    if s1.or().is_1() || s1.fe().is_1() || s1.nf().is_1() || s1.pf().is_1() {
        let _ = uart.d().read();
        let err = if s1.or().is_1() {
            Error::Overrun
        } else if s1.fe().is_1() {
            Error::Framing
        } else if s1.nf().is_1() {
            Error::Noise
        } else {
            Error::Parity
        };
        return Err(nb::Error::Other(err));
    }

    if s1.rdrf().is_0() {
        return Err(nb::Error::WouldBlock);
    }

    Ok(uart.d().read().rt().bits())
}

fn nb_write<UART: sealed::UartInstance>(byte: u8) -> nb::Result<(), Error> {
    let uart = regs::<UART>();
    if uart.s1().read().tdre().is_0() {
        return Err(nb::Error::WouldBlock);
    }
    uart.d().write(|w| unsafe { w.bits(byte) });
    Ok(())
}

fn nb_flush<UART: sealed::UartInstance>() -> nb::Result<(), Error> {
    let uart = regs::<UART>();
    if uart.s1().read().tc().is_0() {
        return Err(nb::Error::WouldBlock);
    }
    Ok(())
}

// ----- embedded_hal_nb::serial impls -----

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::ErrorType for Serial<UART> {
    type Error = Error;
}

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::Read<u8> for Serial<UART> {
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        nb_read::<UART>()
    }
}

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::Write<u8> for Serial<UART> {
    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        nb_write::<UART>(byte)
    }
    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        nb_flush::<UART>()
    }
}

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::ErrorType for Tx<UART> {
    type Error = Error;
}

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::Write<u8> for Tx<UART> {
    fn write(&mut self, byte: u8) -> nb::Result<(), Self::Error> {
        nb_write::<UART>(byte)
    }
    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        nb_flush::<UART>()
    }
}

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::ErrorType for Rx<UART> {
    type Error = Error;
}

impl<UART: sealed::UartInstance> embedded_hal_nb::serial::Read<u8> for Rx<UART> {
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        nb_read::<UART>()
    }
}

// ----- embedded_io impls -----

impl<UART: sealed::UartInstance> embedded_io::ErrorType for Serial<UART> {
    type Error = Error;
}

impl<UART: sealed::UartInstance> embedded_io::Read for Serial<UART> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        buf[0] = nb::block!(nb_read::<UART>())?;
        let mut count = 1;
        while count < buf.len() {
            match nb_read::<UART>() {
                Ok(byte) => {
                    buf[count] = byte;
                    count += 1;
                }
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }
        Ok(count)
    }
}

impl<UART: sealed::UartInstance> embedded_io::Write for Serial<UART> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        nb::block!(nb_write::<UART>(buf[0]))?;
        let mut count = 1;
        while count < buf.len() {
            match nb_write::<UART>(buf[count]) {
                Ok(()) => count += 1,
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }
        Ok(count)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        nb::block!(nb_flush::<UART>())?;
        Ok(())
    }
}

impl<UART: sealed::UartInstance> embedded_io::ErrorType for Tx<UART> {
    type Error = Error;
}

impl<UART: sealed::UartInstance> embedded_io::Write for Tx<UART> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        nb::block!(nb_write::<UART>(buf[0]))?;
        let mut count = 1;
        while count < buf.len() {
            match nb_write::<UART>(buf[count]) {
                Ok(()) => count += 1,
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }
        Ok(count)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        nb::block!(nb_flush::<UART>())?;
        Ok(())
    }
}

impl<UART: sealed::UartInstance> embedded_io::ErrorType for Rx<UART> {
    type Error = Error;
}

impl<UART: sealed::UartInstance> embedded_io::Read for Rx<UART> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        buf[0] = nb::block!(nb_read::<UART>())?;
        let mut count = 1;
        while count < buf.len() {
            match nb_read::<UART>() {
                Ok(byte) => {
                    buf[count] = byte;
                    count += 1;
                }
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }
        Ok(count)
    }
}

// ----- Extension Trait -----

/// Extension trait for creating UART serial ports from PAC peripherals.
///
/// Pin types are constrained by marker traits (e.g., [`Uart0TxPin`]) to
/// ensure only valid pin assignments compile. See `gpio.rs` for the
/// complete pin-peripheral mapping.
pub trait UartExt<TX, RX>: Sized {
    type Instance: sealed::UartInstance;

    fn serial(
        self,
        _tx: TX,
        _rx: RX,
        config: Config,
        clocks: &Clocks,
        sim: &pac::Sim,
    ) -> Serial<Self::Instance>;
}

macro_rules! uart_ext_impl {
    ($PacType:ty, $Instance:ty, $TxPin:ident, $RxPin:ident, $clock_method:ident) => {
        impl<TX: $TxPin, RX: $RxPin> UartExt<TX, RX> for $PacType {
            type Instance = $Instance;

            fn serial(
                self,
                _tx: TX,
                _rx: RX,
                config: Config,
                clocks: &Clocks,
                sim: &pac::Sim,
            ) -> Serial<$Instance> {
                <$Instance as sealed::UartInstance>::enable_clock(sim);
                Serial::<$Instance>::init(config, clocks.$clock_method().raw())
            }
        }
    };
}

uart_ext_impl!(pac::Uart0, Uart0, Uart0TxPin, Uart0RxPin, core_clk);
uart_ext_impl!(pac::Uart1, Uart1, Uart1TxPin, Uart1RxPin, core_clk);
uart_ext_impl!(pac::Uart2, Uart2, Uart2TxPin, Uart2RxPin, bus_clk);

// ----- Async support -----

#[cfg(feature = "async")]
mod async_impl {
    use super::*;
    use embassy_sync::waitqueue::AtomicWaker;

    struct UartWakers {
        rx: AtomicWaker,
        tx: AtomicWaker,
    }

    static UART0_WAKERS: UartWakers = UartWakers {
        rx: AtomicWaker::new(),
        tx: AtomicWaker::new(),
    };
    static UART1_WAKERS: UartWakers = UartWakers {
        rx: AtomicWaker::new(),
        tx: AtomicWaker::new(),
    };
    static UART2_WAKERS: UartWakers = UartWakers {
        rx: AtomicWaker::new(),
        tx: AtomicWaker::new(),
    };

    fn wakers_for(ptr: *const pac::uart0::RegisterBlock) -> &'static UartWakers {
        let addr = ptr as usize;
        if addr == pac::Uart0::PTR as usize {
            &UART0_WAKERS
        } else if addr == pac::Uart1::PTR as usize {
            &UART1_WAKERS
        } else {
            &UART2_WAKERS
        }
    }

    fn on_uart_rx_tx_interrupt<UART: sealed::UartInstance>() {
        let uart = regs::<UART>();
        let s1 = uart.s1().read();
        let wakers = wakers_for(UART::ptr());

        // Wake RX task if data available or error
        if s1.rdrf().is_1() || s1.or().is_1() || s1.fe().is_1() || s1.nf().is_1() || s1.pf().is_1()
        {
            wakers.rx.wake();
        }
        // Wake TX task if transmit buffer empty
        if s1.tdre().is_1() {
            // Disable TIE to prevent repeated firing until re-enabled
            uart.c2().modify(|_, w| w.tie()._0());
            wakers.tx.wake();
        }
        // Wake TX task if transmit complete (for flush)
        if s1.tc().is_1() {
            uart.c2().modify(|_, w| w.tcie()._0());
            wakers.tx.wake();
        }
    }

    /// Call from the `UART0_RX_TX` interrupt handler.
    pub fn on_uart0_rx_tx_interrupt() {
        on_uart_rx_tx_interrupt::<Uart0>();
    }

    /// Call from the `UART1_RX_TX` interrupt handler.
    pub fn on_uart1_rx_tx_interrupt() {
        on_uart_rx_tx_interrupt::<Uart1>();
    }

    /// Call from the `UART2_RX_TX` interrupt handler.
    pub fn on_uart2_rx_tx_interrupt() {
        on_uart_rx_tx_interrupt::<Uart2>();
    }

    // Note: embedded_io_async re-exports embedded_io::ErrorType,
    // so the existing impls (outside this module) already satisfy the requirement.

    // --- embedded_io_async::Read for Rx ---

    impl<UART: sealed::UartInstance> embedded_io_async::Read for Rx<UART> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() {
                return Ok::<usize, Error>(0);
            }
            let wakers = wakers_for(UART::ptr());
            let uart = regs::<UART>();
            // Enable RX interrupt
            uart.c2().modify(|_, w| w.rie()._1());
            let result = core::future::poll_fn(|cx| {
                wakers.rx.register(cx.waker());
                match nb_read::<UART>() {
                    Ok(byte) => core::task::Poll::Ready(Ok(byte)),
                    Err(nb::Error::WouldBlock) => core::task::Poll::Pending,
                    Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
                }
            })
            .await;
            uart.c2().modify(|_, w| w.rie()._0());
            match result {
                Ok(byte) => {
                    buf[0] = byte;
                    // Drain any additional available bytes without blocking
                    let mut count = 1;
                    while count < buf.len() {
                        match nb_read::<UART>() {
                            Ok(byte) => {
                                buf[count] = byte;
                                count += 1;
                            }
                            Err(_) => break,
                        }
                    }
                    Ok(count)
                }
                Err(e) => Err(e),
            }
        }
    }

    // --- embedded_io_async::Write for Tx ---

    impl<UART: sealed::UartInstance> embedded_io_async::Write for Tx<UART> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() {
                return Ok::<usize, Error>(0);
            }
            let wakers = wakers_for(UART::ptr());
            let uart = regs::<UART>();
            // Write first byte (may need to wait for TDRE)
            core::future::poll_fn(|cx| {
                wakers.tx.register(cx.waker());
                match nb_write::<UART>(buf[0]) {
                    Ok(()) => core::task::Poll::Ready(Ok(())),
                    Err(nb::Error::WouldBlock) => {
                        uart.c2().modify(|_, w| w.tie()._1());
                        core::task::Poll::Pending
                    }
                    Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
                }
            })
            .await?;
            // Try to write remaining bytes without blocking
            let mut count = 1;
            while count < buf.len() {
                match nb_write::<UART>(buf[count]) {
                    Ok(()) => count += 1,
                    Err(_) => break,
                }
            }
            Ok(count)
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            let wakers = wakers_for(UART::ptr());
            let uart = regs::<UART>();
            core::future::poll_fn(|cx| {
                wakers.tx.register(cx.waker());
                match nb_flush::<UART>() {
                    Ok(()) => core::task::Poll::Ready(Ok(())),
                    Err(nb::Error::WouldBlock) => {
                        uart.c2().modify(|_, w| w.tcie()._1());
                        core::task::Poll::Pending
                    }
                    Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
                }
            })
            .await
        }
    }

    // --- embedded_io_async::Read/Write for Serial ---

    impl<UART: sealed::UartInstance> embedded_io_async::Read for Serial<UART> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() {
                return Ok::<usize, Error>(0);
            }
            let wakers = wakers_for(UART::ptr());
            let uart = regs::<UART>();
            uart.c2().modify(|_, w| w.rie()._1());
            let result = core::future::poll_fn(|cx| {
                wakers.rx.register(cx.waker());
                match nb_read::<UART>() {
                    Ok(byte) => core::task::Poll::Ready(Ok(byte)),
                    Err(nb::Error::WouldBlock) => core::task::Poll::Pending,
                    Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
                }
            })
            .await;
            uart.c2().modify(|_, w| w.rie()._0());
            match result {
                Ok(byte) => {
                    buf[0] = byte;
                    let mut count = 1;
                    while count < buf.len() {
                        match nb_read::<UART>() {
                            Ok(byte) => {
                                buf[count] = byte;
                                count += 1;
                            }
                            Err(_) => break,
                        }
                    }
                    Ok(count)
                }
                Err(e) => Err(e),
            }
        }
    }

    impl<UART: sealed::UartInstance> embedded_io_async::Write for Serial<UART> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() {
                return Ok(0);
            }
            let wakers = wakers_for(UART::ptr());
            let uart = regs::<UART>();
            core::future::poll_fn(|cx| {
                wakers.tx.register(cx.waker());
                match nb_write::<UART>(buf[0]) {
                    Ok(()) => core::task::Poll::Ready(Ok(())),
                    Err(nb::Error::WouldBlock) => {
                        uart.c2().modify(|_, w| w.tie()._1());
                        core::task::Poll::Pending
                    }
                    Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
                }
            })
            .await?;
            let mut count = 1;
            while count < buf.len() {
                match nb_write::<UART>(buf[count]) {
                    Ok(()) => count += 1,
                    Err(_) => break,
                }
            }
            Ok(count)
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            let wakers = wakers_for(UART::ptr());
            let uart = regs::<UART>();
            core::future::poll_fn(|cx| {
                wakers.tx.register(cx.waker());
                match nb_flush::<UART>() {
                    Ok(()) => core::task::Poll::Ready(Ok(())),
                    Err(nb::Error::WouldBlock) => {
                        uart.c2().modify(|_, w| w.tcie()._1());
                        core::task::Poll::Pending
                    }
                    Err(nb::Error::Other(e)) => core::task::Poll::Ready(Err(e)),
                }
            })
            .await
        }
    }
}

#[cfg(feature = "async")]
pub use async_impl::{on_uart0_rx_tx_interrupt, on_uart1_rx_tx_interrupt, on_uart2_rx_tx_interrupt};
