use core::marker::PhantomData;

use crate::clocks::Clocks;
use crate::gpio::{Alternate, Pin};
use crate::pac;
use crate::time::Hertz;

use embedded_hal::i2c::{ErrorKind, ErrorType, NoAcknowledgeSource, Operation, SevenBitAddress};

// ----- Configuration -----

/// I2C bus configuration.
pub struct Config {
    pub frequency: Hertz,
}

impl Config {
    /// Create a new configuration with the given SCL frequency.
    pub fn new(frequency: Hertz) -> Self {
        Config { frequency }
    }
}

// ----- Error -----

/// I2C communication error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Arbitration was lost during communication.
    ArbitrationLoss,
    /// Slave did not acknowledge its address.
    AddressNack,
    /// Slave did not acknowledge a data byte.
    DataNack,
}

impl embedded_hal::i2c::Error for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::ArbitrationLoss => ErrorKind::ArbitrationLoss,
            Error::AddressNack => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
            Error::DataNack => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data),
        }
    }
}

// ----- Instance Abstraction -----

mod sealed {
    pub trait I2cInstance {
        fn ptr() -> *const crate::pac::i2c0::RegisterBlock;
        fn enable_clock(sim: &crate::pac::Sim);
    }
}

/// Marker type for I2C0.
pub struct I2c0;

impl sealed::I2cInstance for I2c0 {
    fn ptr() -> *const pac::i2c0::RegisterBlock {
        pac::I2c0::PTR
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc4().modify(|_, w| w.i2c0()._1());
    }
}

/// Marker type for I2C1 (mk20d7 only).
#[cfg(feature = "mk20d7")]
pub struct I2c1;

#[cfg(feature = "mk20d7")]
impl sealed::I2cInstance for I2c1 {
    fn ptr() -> *const pac::i2c0::RegisterBlock {
        // Safety: I2C1 and I2C0 have identical register layouts.
        pac::I2c1::PTR as *const pac::i2c0::RegisterBlock
    }
    fn enable_clock(sim: &pac::Sim) {
        sim.scgc4().modify(|_, w| w.i2c1()._1());
    }
}

fn regs<I2C: sealed::I2cInstance>() -> &'static pac::i2c0::RegisterBlock {
    unsafe { &*I2C::ptr() }
}

// ----- Driver Type -----

/// I2C master driver.
pub struct I2c<I2C> {
    _i2c: PhantomData<I2C>,
}

// ----- Baud Rate Calculation -----

/// ICR divider table from K20 reference manual (Table 46-41).
/// Index = ICR field value (0-63), value = SCL clock divider.
const ICR_DIVIDERS: [u16; 64] = [
    20, 22, 24, 26, 28, 30, 34, 40,
    28, 32, 36, 40, 44, 48, 56, 68,
    48, 56, 64, 72, 80, 88, 104, 128,
    80, 96, 112, 128, 144, 160, 192, 240,
    160, 192, 224, 256, 288, 320, 384, 480,
    320, 384, 448, 512, 576, 640, 768, 960,
    640, 768, 896, 1024, 1152, 1280, 1536, 1920,
    1280, 1536, 1792, 2048, 2304, 2560, 3072, 3840,
];

const MULT_VALUES: [u16; 3] = [1, 2, 4];

/// Select ICR and MULT for the closest SCL frequency not exceeding `target`.
///
/// `SCL_freq = bus_clk / (MULT × ICR_divider[ICR])`
fn calc_frequency(bus_clk: u32, target: u32) -> (u8, u8) {
    let mut best_icr: u8 = 0;
    let mut best_mult: u8 = 0;
    let mut best_freq: u32 = 0;

    for (mult_idx, &mult) in MULT_VALUES.iter().enumerate() {
        for (icr_idx, &divider) in ICR_DIVIDERS.iter().enumerate() {
            let freq = bus_clk / (mult as u32 * divider as u32);
            if freq <= target && freq > best_freq {
                best_freq = freq;
                best_icr = icr_idx as u8;
                best_mult = mult_idx as u8;
            }
        }
    }

    (best_icr, best_mult)
}

// ----- Initialization -----

impl<I2C: sealed::I2cInstance> I2c<I2C> {
    fn init(config: Config, bus_clk: u32) -> Self {
        let i2c = regs::<I2C>();
        let (icr, mult) = calc_frequency(bus_clk, config.frequency.raw());

        // Disable I2C during configuration
        i2c.c1().write(|w| w.iicen()._0());

        // Set frequency divider
        i2c.f().write(|w| {
            let w = unsafe { w.icr().bits(icr) };
            match mult {
                0 => w.mult()._00(),
                1 => w.mult()._01(),
                _ => w.mult()._10(),
            }
        });

        // Clear pending status flags (w1c — use write, not modify)
        i2c.s().write(|w| w.iicif()._1().arbl()._1());

        // Enable I2C (slave mode initially; master mode set on START)
        i2c.c1().write(|w| w.iicen()._1());

        I2c { _i2c: PhantomData }
    }
}

// ----- Protocol Helpers -----

impl<I2C: sealed::I2cInstance> I2c<I2C> {
    /// Poll for transfer complete and check for arbitration loss.
    fn wait_transfer(&self) -> Result<(), Error> {
        let i2c = regs::<I2C>();
        loop {
            let s = i2c.s().read();
            if s.iicif().is_1() {
                if s.arbl().is_1() {
                    i2c.s().write(|w| w.arbl()._1().iicif()._1());
                    return Err(Error::ArbitrationLoss);
                }
                i2c.s().write(|w| w.iicif()._1());
                return Ok(());
            }
        }
    }

    /// Generate START condition and send address byte.
    fn start(&self, addr: u8, read: bool) -> Result<(), Error> {
        let i2c = regs::<I2C>();

        // Wait for bus idle
        while i2c.s().read().busy().is_1() {}

        // MST 0→1 generates START; set TX for address byte
        i2c.c1().write(|w| w.iicen()._1().mst()._1().tx()._1());

        // Send address byte (7-bit address + R/W bit)
        let addr_byte = (addr << 1) | if read { 1 } else { 0 };
        i2c.d().write(|w| unsafe { w.data().bits(addr_byte) });

        self.wait_transfer()?;

        if i2c.s().read().rxak().is_1() {
            return Err(Error::AddressNack);
        }
        Ok(())
    }

    /// Generate REPEATED START condition and send address byte.
    fn repeated_start(&self, addr: u8, read: bool) -> Result<(), Error> {
        let i2c = regs::<I2C>();

        // RSTA generates repeated START; keep IICEN, MST, TX
        i2c.c1().write(|w| {
            w.iicen()._1().mst()._1().tx()._1().rsta().set_bit()
        });

        let addr_byte = (addr << 1) | if read { 1 } else { 0 };
        i2c.d().write(|w| unsafe { w.data().bits(addr_byte) });

        self.wait_transfer()?;

        if i2c.s().read().rxak().is_1() {
            return Err(Error::AddressNack);
        }
        Ok(())
    }

    /// Generate STOP condition (MST 1→0).
    fn stop(&self) {
        let i2c = regs::<I2C>();
        i2c.c1().write(|w| w.iicen()._1());
    }

    /// Write data bytes in master transmit mode.
    fn write_bytes(&self, bytes: &[u8]) -> Result<(), Error> {
        let i2c = regs::<I2C>();

        for &byte in bytes {
            i2c.d().write(|w| unsafe { w.data().bits(byte) });
            self.wait_transfer()?;
            if i2c.s().read().rxak().is_1() {
                return Err(Error::DataNack);
            }
        }

        Ok(())
    }

    /// Read bytes across consecutive Read operations in master receive mode.
    ///
    /// Handles ACK/NACK sequencing per the Kinetis I2C protocol:
    /// - ACK all bytes except the last in the group
    /// - NACK the last byte to tell the slave to release SDA
    /// - If `generate_stop`: clear MST before reading last byte (STOP)
    /// - If not: switch to TX before reading last byte (for upcoming RSTA)
    fn read_group(&self, ops: &mut [Operation<'_>], generate_stop: bool) -> Result<(), Error> {
        let i2c = regs::<I2C>();

        // Count total bytes across all Read operations in this group
        let total: usize = ops
            .iter()
            .map(|op| match op {
                Operation::Read(buf) => buf.len(),
                _ => 0,
            })
            .sum();

        if total == 0 {
            if generate_stop {
                self.stop();
            }
            return Ok(());
        }

        // Switch to RX mode with initial ACK/NACK setting
        if total == 1 {
            // Single byte: NACK immediately
            i2c.c1()
                .write(|w| w.iicen()._1().mst()._1().txak()._1());
        } else {
            // Multiple bytes: ACK first
            i2c.c1().write(|w| w.iicen()._1().mst()._1());
        }

        // Dummy read to trigger first receive
        let _ = i2c.d().read();

        let mut global = 0usize;
        for op in ops.iter_mut() {
            if let Operation::Read(buf) = op {
                for byte in buf.iter_mut() {
                    self.wait_transfer()?;

                    // Set NACK for the upcoming last byte's receive
                    if total > 1 && global == total - 2 {
                        i2c.c1()
                            .write(|w| w.iicen()._1().mst()._1().txak()._1());
                    }

                    // For the last byte: prevent another receive after reading D
                    if global == total - 1 {
                        if generate_stop {
                            // Clear MST → STOP; reading D won't trigger another receive
                            self.stop();
                        } else {
                            // Switch to TX → reading D won't trigger another receive;
                            // stays in master mode for upcoming RSTA
                            i2c.c1()
                                .write(|w| w.iicen()._1().mst()._1().tx()._1());
                        }
                    }

                    *byte = i2c.d().read().data().bits();
                    global += 1;
                }
            }
        }

        Ok(())
    }
}

// ----- Transaction Implementation -----

impl<I2C: sealed::I2cInstance> I2c<I2C> {
    /// Execute a transaction, grouping consecutive same-direction operations.
    ///
    /// Per the embedded-hal contract:
    /// - Adjacent operations of the same type are sent without SR or SP between them
    /// - Adjacent operations of different types get SR + SAD+R/W between them
    /// - SP is sent after the last operation
    fn do_transaction(
        &mut self,
        address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Error> {
        let mut i = 0;

        while i < operations.len() {
            let is_read = matches!(operations[i], Operation::Read(_));

            // Find end of consecutive same-direction operations
            let mut group_end = i + 1;
            while group_end < operations.len()
                && matches!(operations[group_end], Operation::Read(_)) == is_read
            {
                group_end += 1;
            }

            let is_last_group = group_end == operations.len();

            // START or REPEATED START + address
            if i == 0 {
                self.start(address, is_read)?;
            } else {
                self.repeated_start(address, is_read)?;
            }

            if is_read {
                self.read_group(&mut operations[i..group_end], is_last_group)?;
            } else {
                for op in &operations[i..group_end] {
                    if let Operation::Write(bytes) = op {
                        self.write_bytes(bytes)?;
                    }
                }
                if is_last_group {
                    self.stop();
                }
            }

            i = group_end;
        }

        Ok(())
    }
}

// ----- embedded_hal::i2c impls -----

impl<I2C: sealed::I2cInstance> ErrorType for I2c<I2C> {
    type Error = Error;
}

impl<I2C: sealed::I2cInstance> embedded_hal::i2c::I2c<SevenBitAddress> for I2c<I2C> {
    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        if operations.is_empty() {
            return Ok(());
        }

        let result = self.do_transaction(address, operations);

        if result.is_err() {
            // Ensure bus is released on error
            self.stop();
        }

        result
    }
}

// ----- Extension Trait -----

/// Extension trait for creating I2C drivers from PAC I2C peripherals.
pub trait I2cExt: Sized {
    type Instance: sealed::I2cInstance;

    fn i2c<const SP: char, const SN: u8, const DP: char, const DN: u8>(
        self,
        _scl: Pin<SP, SN, Alternate<2>>,
        _sda: Pin<DP, DN, Alternate<2>>,
        config: Config,
        clocks: &Clocks,
        sim: &pac::Sim,
    ) -> I2c<Self::Instance>;
}

impl I2cExt for pac::I2c0 {
    type Instance = I2c0;

    fn i2c<const SP: char, const SN: u8, const DP: char, const DN: u8>(
        self,
        _scl: Pin<SP, SN, Alternate<2>>,
        _sda: Pin<DP, DN, Alternate<2>>,
        config: Config,
        clocks: &Clocks,
        sim: &pac::Sim,
    ) -> I2c<I2c0> {
        <I2c0 as sealed::I2cInstance>::enable_clock(sim);
        I2c::<I2c0>::init(config, clocks.bus_clk().raw())
    }
}

#[cfg(feature = "mk20d7")]
impl I2cExt for pac::I2c1 {
    type Instance = I2c1;

    fn i2c<const SP: char, const SN: u8, const DP: char, const DN: u8>(
        self,
        _scl: Pin<SP, SN, Alternate<2>>,
        _sda: Pin<DP, DN, Alternate<2>>,
        config: Config,
        clocks: &Clocks,
        sim: &pac::Sim,
    ) -> I2c<I2c1> {
        <I2c1 as sealed::I2cInstance>::enable_clock(sim);
        I2c::<I2c1>::init(config, clocks.bus_clk().raw())
    }
}

// ----- Async support -----

#[cfg(feature = "async")]
mod async_impl {
    use super::*;
    use embassy_sync::waitqueue::AtomicWaker;

    static I2C0_WAKER: AtomicWaker = AtomicWaker::new();
    #[cfg(feature = "mk20d7")]
    static I2C1_WAKER: AtomicWaker = AtomicWaker::new();

    fn waker_for(ptr: *const pac::i2c0::RegisterBlock) -> &'static AtomicWaker {
        if ptr as usize == pac::I2c0::PTR as usize {
            &I2C0_WAKER
        } else {
            #[cfg(feature = "mk20d7")]
            {
                &I2C1_WAKER
            }
            #[cfg(not(feature = "mk20d7"))]
            {
                &I2C0_WAKER
            }
        }
    }

    fn on_i2c_interrupt(
        i2c: &pac::i2c0::RegisterBlock,
        waker: &AtomicWaker,
    ) {
        if i2c.s().read().iicif().is_1() {
            // Disable interrupt to prevent re-triggering.
            // Driver will read status and clear IICIF.
            i2c.c1().modify(|_, w| w.iicie()._0());
            waker.wake();
        }
    }

    /// Call from the `I2C0` interrupt handler.
    pub fn on_i2c0_interrupt() {
        on_i2c_interrupt(unsafe { &*pac::I2c0::PTR }, &I2C0_WAKER);
    }

    /// Call from the `I2C1` interrupt handler (mk20d7 only).
    #[cfg(feature = "mk20d7")]
    pub fn on_i2c1_interrupt() {
        on_i2c_interrupt(
            unsafe { &*(pac::I2c1::PTR as *const pac::i2c0::RegisterBlock) },
            &I2C1_WAKER,
        );
    }

    // --- Async protocol helpers ---

    impl<I2C: sealed::I2cInstance> I2c<I2C> {
        /// Async version of wait_transfer: enables IICIE, awaits IICIF, checks ARBL.
        async fn wait_transfer_async(&self) -> Result<(), Error> {
            let i2c = regs::<I2C>();
            let waker = waker_for(I2C::ptr());

            // Enable I2C interrupt
            i2c.c1().modify(|_, w| w.iicie()._1());

            core::future::poll_fn(|cx| {
                waker.register(cx.waker());
                let s = i2c.s().read();
                if s.iicif().is_1() {
                    if s.arbl().is_1() {
                        i2c.s().write(|w| w.arbl()._1().iicif()._1());
                        core::task::Poll::Ready(Err(Error::ArbitrationLoss))
                    } else {
                        i2c.s().write(|w| w.iicif()._1());
                        core::task::Poll::Ready(Ok(()))
                    }
                } else {
                    // Re-enable interrupt (ISR disabled it on spurious wake)
                    i2c.c1().modify(|_, w| w.iicie()._1());
                    core::task::Poll::Pending
                }
            })
            .await
        }

        async fn start_async(&self, addr: u8, read: bool) -> Result<(), Error> {
            let i2c = regs::<I2C>();
            // Spin-wait for bus idle (no interrupt for this, transitions are fast)
            while i2c.s().read().busy().is_1() {}
            i2c.c1().write(|w| w.iicen()._1().mst()._1().tx()._1());
            let addr_byte = (addr << 1) | if read { 1 } else { 0 };
            i2c.d().write(|w| unsafe { w.data().bits(addr_byte) });
            self.wait_transfer_async().await?;
            if i2c.s().read().rxak().is_1() {
                return Err(Error::AddressNack);
            }
            Ok(())
        }

        async fn repeated_start_async(&self, addr: u8, read: bool) -> Result<(), Error> {
            let i2c = regs::<I2C>();
            i2c.c1().write(|w| {
                w.iicen()._1().mst()._1().tx()._1().rsta().set_bit()
            });
            let addr_byte = (addr << 1) | if read { 1 } else { 0 };
            i2c.d().write(|w| unsafe { w.data().bits(addr_byte) });
            self.wait_transfer_async().await?;
            if i2c.s().read().rxak().is_1() {
                return Err(Error::AddressNack);
            }
            Ok(())
        }

        async fn write_bytes_async(&self, bytes: &[u8]) -> Result<(), Error> {
            let i2c = regs::<I2C>();
            for &byte in bytes {
                i2c.d().write(|w| unsafe { w.data().bits(byte) });
                self.wait_transfer_async().await?;
                if i2c.s().read().rxak().is_1() {
                    return Err(Error::DataNack);
                }
            }
            Ok(())
        }

        async fn read_group_async(
            &self,
            ops: &mut [Operation<'_>],
            generate_stop: bool,
        ) -> Result<(), Error> {
            let i2c = regs::<I2C>();

            let total: usize = ops
                .iter()
                .map(|op| match op {
                    Operation::Read(buf) => buf.len(),
                    _ => 0,
                })
                .sum();

            if total == 0 {
                if generate_stop {
                    self.stop();
                }
                return Ok(());
            }

            if total == 1 {
                i2c.c1().write(|w| w.iicen()._1().mst()._1().txak()._1());
            } else {
                i2c.c1().write(|w| w.iicen()._1().mst()._1());
            }

            // Dummy read to trigger first receive
            let _ = i2c.d().read();

            let mut global = 0usize;
            for op in ops.iter_mut() {
                if let Operation::Read(buf) = op {
                    for byte in buf.iter_mut() {
                        self.wait_transfer_async().await?;

                        if total > 1 && global == total - 2 {
                            i2c.c1()
                                .write(|w| w.iicen()._1().mst()._1().txak()._1());
                        }

                        if global == total - 1 {
                            if generate_stop {
                                self.stop();
                            } else {
                                i2c.c1()
                                    .write(|w| w.iicen()._1().mst()._1().tx()._1());
                            }
                        }

                        *byte = i2c.d().read().data().bits();
                        global += 1;
                    }
                }
            }

            Ok(())
        }

        async fn do_transaction_async(
            &mut self,
            address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Error> {
            let mut i = 0;

            while i < operations.len() {
                let is_read = matches!(operations[i], Operation::Read(_));

                let mut group_end = i + 1;
                while group_end < operations.len()
                    && matches!(operations[group_end], Operation::Read(_)) == is_read
                {
                    group_end += 1;
                }

                let is_last_group = group_end == operations.len();

                if i == 0 {
                    self.start_async(address, is_read).await?;
                } else {
                    self.repeated_start_async(address, is_read).await?;
                }

                if is_read {
                    self.read_group_async(&mut operations[i..group_end], is_last_group)
                        .await?;
                } else {
                    for op in &operations[i..group_end] {
                        if let Operation::Write(bytes) = op {
                            self.write_bytes_async(bytes).await?;
                        }
                    }
                    if is_last_group {
                        self.stop();
                    }
                }

                i = group_end;
            }

            Ok(())
        }
    }

    // --- embedded_hal_async::i2c::I2c ---

    impl<I2C: sealed::I2cInstance> embedded_hal_async::i2c::I2c<SevenBitAddress> for I2c<I2C> {
        async fn transaction(
            &mut self,
            address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            if operations.is_empty() {
                return Ok(());
            }

            let result = self.do_transaction_async(address, operations).await;

            if result.is_err() {
                self.stop();
            }

            result
        }
    }
}

#[cfg(feature = "async")]
pub use async_impl::on_i2c0_interrupt;
#[cfg(all(feature = "async", feature = "mk20d7"))]
pub use async_impl::on_i2c1_interrupt;
