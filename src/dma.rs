//! eDMA (Enhanced Direct Memory Access) driver with DMAMUX support.
//!
//! The eDMA engine performs CPU-free data transfers between memory and
//! peripherals. The DMAMUX routes peripheral request signals to DMA channels.
//!
//! - MK20D5 (Teensy 3.0): 4 channels
//! - MK20D7 (Teensy 3.1/3.2): 16 channels
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use mk20dx_hal::dma::{DmaSource, TransferConfig, TransferSize};
//!
//! let dp = pac::Peripherals::take().unwrap();
//! // ... configure clocks, disable watchdog ...
//! let mut dma = dp.DMA.split(dp.DMAMUX, &dp.SIM);
//!
//! // Memory-to-memory transfer via software trigger
//! let src = [1u8, 2, 3, 4];
//! let mut dst = [0u8; 4];
//! unsafe {
//!     dma.ch0.configure_memcpy(src.as_ptr(), dst.as_mut_ptr(), 4);
//! }
//! dma.ch0.start();
//! while !dma.ch0.is_complete() {}
//! dma.ch0.clear_done();
//! ```

use crate::pac;

#[cfg(feature = "mk20d5")]
const NUM_CHANNELS: usize = 4;
#[cfg(feature = "mk20d7")]
const NUM_CHANNELS: usize = 16;

/// Access the DMA register block.
fn dma_regs() -> &'static pac::dma::RegisterBlock {
    // SAFETY: PTR is a valid pointer to the DMA register block,
    // which is memory-mapped I/O that exists for the program's lifetime.
    unsafe { &*pac::Dma::PTR }
}

/// Access the DMAMUX register block.
fn dmamux_regs() -> &'static pac::dmamux::RegisterBlock {
    // SAFETY: PTR is a valid pointer to the DMAMUX register block.
    unsafe { &*pac::Dmamux::PTR }
}

/// Map channel number to DCHPRI array index.
///
/// DCHPRI registers are byte-swapped within 32-bit groups:
/// ch0→idx3, ch1→idx2, ch2→idx1, ch3→idx0, ch4→idx7, etc.
pub const fn dchpri_index(ch: u8) -> usize {
    (ch ^ 3) as usize
}

// ----- Transfer Size -----

/// DMA transfer data size.
///
/// Maps directly to the TCD ATTR SSIZE/DSIZE encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferSize {
    /// 8-bit (1 byte) transfers.
    Bits8,
    /// 16-bit (2 byte) transfers.
    Bits16,
    /// 32-bit (4 byte) transfers.
    Bits32,
    /// 16-byte burst transfers.
    Burst16,
}

impl TransferSize {
    /// Number of bytes per transfer unit.
    pub const fn bytes(self) -> u32 {
        match self {
            TransferSize::Bits8 => 1,
            TransferSize::Bits16 => 2,
            TransferSize::Bits32 => 4,
            TransferSize::Burst16 => 16,
        }
    }
}

// ----- Transfer Config -----

/// TCD (Transfer Control Descriptor) configuration.
///
/// Describes a complete DMA transfer: source/destination addresses, data sizes,
/// address offsets per transfer, minor loop byte count, and major loop count.
pub struct TransferConfig {
    /// Source address (must be aligned to `source_size`).
    pub source_addr: u32,
    /// Destination address (must be aligned to `dest_size`).
    pub dest_addr: u32,
    /// Source data transfer size.
    pub source_size: TransferSize,
    /// Destination data transfer size.
    pub dest_size: TransferSize,
    /// Signed offset applied to source address after each read.
    pub source_offset: i16,
    /// Signed offset applied to destination address after each write.
    pub dest_offset: i16,
    /// Number of bytes transferred per DMA activation (minor loop).
    pub minor_loop_bytes: u32,
    /// Number of minor loop iterations in the major loop (must be >= 1).
    pub major_loop_count: u16,
    /// Signed adjustment to source address after major loop completion.
    pub source_last_adjust: i32,
    /// Signed adjustment to destination address after major loop completion.
    pub dest_last_adjust: i32,
    /// Destination address modulo (0 = disabled, 1-31 = wrap at 2^N bytes).
    /// Enables circular buffer behavior for the destination address.
    pub dest_modulo: u8,
    /// If true (default), auto-disable hardware request on major loop complete.
    /// Set to false for continuous/circular transfers.
    pub auto_disable: bool,
}

// ----- DMA Source -----

/// DMAMUX request source identifier.
///
/// Routes a peripheral's DMA request signal to a DMA channel.
/// Use the named constants (e.g., `DmaSource::UART0_RX`) or construct
/// from a raw slot number with [`DmaSource::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaSource(u8);

impl DmaSource {
    /// Create a DmaSource from a raw DMAMUX slot number (0-63).
    pub const fn new(slot: u8) -> Self {
        Self(slot & 0x3F)
    }

    /// Get the raw DMAMUX slot number.
    pub const fn slot(self) -> u8 {
        self.0
    }

    /// Disabled (no DMA request source).
    pub const DISABLED: Self = Self(0);

    // UART
    pub const UART0_RX: Self = Self(2);
    pub const UART0_TX: Self = Self(3);
    pub const UART1_RX: Self = Self(4);
    pub const UART1_TX: Self = Self(5);
    pub const UART2_RX: Self = Self(6);
    pub const UART2_TX: Self = Self(7);

    // I2S
    pub const I2S0_RX: Self = Self(14);
    pub const I2S0_TX: Self = Self(15);

    // SPI
    pub const SPI0_RX: Self = Self(16);
    pub const SPI0_TX: Self = Self(17);
    #[cfg(feature = "mk20d7")]
    pub const SPI1_RX: Self = Self(18);
    #[cfg(feature = "mk20d7")]
    pub const SPI1_TX: Self = Self(19);

    // I2C
    pub const I2C0: Self = Self(22);
    #[cfg(feature = "mk20d7")]
    pub const I2C1: Self = Self(23);

    // FTM0 channels
    pub const FTM0_CH0: Self = Self(24);
    pub const FTM0_CH1: Self = Self(25);
    pub const FTM0_CH2: Self = Self(26);
    pub const FTM0_CH3: Self = Self(27);
    pub const FTM0_CH4: Self = Self(28);
    pub const FTM0_CH5: Self = Self(29);
    pub const FTM0_CH6: Self = Self(30);
    pub const FTM0_CH7: Self = Self(31);

    // FTM1 channels
    pub const FTM1_CH0: Self = Self(32);
    pub const FTM1_CH1: Self = Self(33);

    // FTM2 channels (mk20d7 only)
    #[cfg(feature = "mk20d7")]
    pub const FTM2_CH0: Self = Self(34);
    #[cfg(feature = "mk20d7")]
    pub const FTM2_CH1: Self = Self(35);

    // ADC
    pub const ADC0: Self = Self(40);
    #[cfg(feature = "mk20d7")]
    pub const ADC1: Self = Self(41);

    // Comparators
    pub const CMP0: Self = Self(42);
    pub const CMP1: Self = Self(43);
    #[cfg(feature = "mk20d7")]
    pub const CMP2: Self = Self(44);

    // DAC
    pub const DAC0: Self = Self(45);

    // Other
    pub const CMT: Self = Self(47);
    pub const PDB: Self = Self(48);

    // Port pin interrupts
    pub const PORT_A: Self = Self(49);
    pub const PORT_B: Self = Self(50);
    pub const PORT_C: Self = Self(51);
    pub const PORT_D: Self = Self(52);
    pub const PORT_E: Self = Self(53);

    // Always-on (no peripheral throttle — for software/mem-to-mem)
    pub const ALWAYS_ON0: Self = Self(54);
    pub const ALWAYS_ON1: Self = Self(55);
    pub const ALWAYS_ON2: Self = Self(56);
    pub const ALWAYS_ON3: Self = Self(57);
    pub const ALWAYS_ON4: Self = Self(58);
    pub const ALWAYS_ON5: Self = Self(59);
    pub const ALWAYS_ON6: Self = Self(60);
    pub const ALWAYS_ON7: Self = Self(61);
    pub const ALWAYS_ON8: Self = Self(62);
    pub const ALWAYS_ON9: Self = Self(63);
}

// ----- Channel Linking -----

/// DMA channel linking mode.
///
/// Channel linking triggers another DMA channel at the end of each minor loop
/// iteration (minor linking), at the end of the major loop (major linking),
/// or both. Minor linking reduces the maximum `major_loop_count` from 32767
/// to 511.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelLink {
    /// No channel linking (default behavior).
    None,
    /// Trigger `channel` after each minor loop iteration.
    MinorLoop { channel: u8 },
    /// Trigger `channel` after major loop completes.
    MajorLoop { channel: u8 },
    /// Minor loop triggers `minor_channel`, major loop triggers `major_channel`.
    Both { minor_channel: u8, major_channel: u8 },
}

// ----- Scatter-Gather TCD -----

/// A scatter-gather TCD descriptor matching the hardware layout.
///
/// Must be 32-byte aligned. When `CSR.ESG=1`, the DMA engine loads the
/// next TCD from the address in `dlastsga` after major loop completion,
/// enabling automatic descriptor chaining.
///
/// For circular chains, set the last TCD's `dlastsga` to point back to
/// the first TCD.
///
/// `CSR.ESG` and `CSR.DREQ` are mutually exclusive per the reference
/// manual — when ESG=1, DREQ is ignored.
#[repr(C, align(32))]
#[derive(Clone, Copy)]
pub struct ScatterGatherTcd {
    pub saddr: u32,
    pub soff: u16,
    pub attr: u16,
    pub nbytes: u32,
    pub slast: u32,
    pub daddr: u32,
    pub doff: u16,
    pub citer: u16,
    pub dlastsga: u32,
    pub csr: u16,
    pub biter: u16,
}

impl ScatterGatherTcd {
    /// Create a zeroed TCD.
    pub const fn new() -> Self {
        Self {
            saddr: 0,
            soff: 0,
            attr: 0,
            nbytes: 0,
            slast: 0,
            daddr: 0,
            doff: 0,
            citer: 0,
            dlastsga: 0,
            csr: 0,
            biter: 0,
        }
    }

    /// Encode ATTR field from source and destination transfer sizes.
    pub const fn attr_from_sizes(source: TransferSize, dest: TransferSize) -> u16 {
        let ssize = match source {
            TransferSize::Bits8 => 0,
            TransferSize::Bits16 => 1,
            TransferSize::Bits32 => 2,
            TransferSize::Burst16 => 4,
        };
        let dsize = match dest {
            TransferSize::Bits8 => 0,
            TransferSize::Bits16 => 1,
            TransferSize::Bits32 => 2,
            TransferSize::Burst16 => 4,
        };
        (ssize << 8) | dsize
    }

    /// Build a TCD from a `TransferConfig`.
    ///
    /// Sets ESG=1 in CSR for scatter-gather mode. The caller must set
    /// `dlastsga` to point to the next TCD in the chain.
    pub fn from_config(config: &TransferConfig) -> Self {
        let attr = Self::attr_from_sizes(config.source_size, config.dest_size)
            | ((config.dest_modulo as u16 & 0x1F) << 3);
        Self {
            saddr: config.source_addr,
            soff: config.source_offset as u16,
            attr,
            nbytes: config.minor_loop_bytes,
            slast: config.source_last_adjust as u32,
            daddr: config.dest_addr,
            doff: config.dest_offset as u16,
            citer: config.major_loop_count,
            dlastsga: 0,
            // ESG=1 (bit 4), DREQ based on auto_disable
            csr: (1 << 4) | if config.auto_disable { 1 << 3 } else { 0 },
            biter: config.major_loop_count,
        }
    }

    /// Set the next TCD address for scatter-gather chaining.
    pub fn set_next(&mut self, next: *const ScatterGatherTcd) {
        self.dlastsga = next as u32;
    }
}

// ----- DMA Error -----

/// DMA transfer error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DmaError {
    /// Source address not properly aligned for transfer size.
    SourceAddressError,
    /// Source offset not consistent with transfer size.
    SourceOffsetError,
    /// Destination address not properly aligned for transfer size.
    DestAddressError,
    /// Destination offset not consistent with transfer size.
    DestOffsetError,
    /// NBYTES or CITER configuration invalid.
    NbytesConfigError,
    /// Scatter/gather address not on 32-byte boundary.
    ScatterGatherError,
    /// Bus error on source read.
    SourceBusError,
    /// Bus error on destination write.
    DestBusError,
    /// Transfer was cancelled.
    Cancelled,
}

// ----- DMA Channel -----

/// A single eDMA channel.
///
/// Zero-sized type parameterized by channel number. All state lives in hardware
/// registers. Provides methods for TCD configuration, transfer control,
/// status checking, and interrupt management.
pub struct DmaChannel<const CH: u8>;

impl<const CH: u8> DmaChannel<CH> {
    // --- DMAMUX Source Routing ---

    /// Set the DMAMUX request source for this channel.
    ///
    /// Disables the channel in the DMAMUX, writes the new source, then
    /// re-enables it. The hardware requires the channel to be disabled
    /// before changing the source.
    pub fn set_source(&mut self, source: DmaSource) {
        let dmamux = dmamux_regs();
        let ch = CH as usize;
        // Disable channel before changing source
        dmamux.chcfg(ch).write(|w| w);
        // Write source and enable
        // SAFETY: source is a 6-bit field (DmaSource masks to 0x3F).
        dmamux.chcfg(ch).write(|w| {
            unsafe { w.source().bits(source.0) }
                .enbl()._1()
        });
    }

    /// Disable the DMAMUX source for this channel (stops hardware requests).
    pub fn disable_source(&mut self) {
        dmamux_regs().chcfg(CH as usize).write(|w| w);
    }

    // --- TCD Configuration ---

    /// Configure the TCD for a transfer.
    ///
    /// Writes all TCD registers for this channel. Sets DREQ=1 so the hardware
    /// request is automatically disabled when the major loop completes.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - Source and destination addresses are valid and properly aligned for
    ///   their respective transfer sizes.
    /// - Memory regions remain valid for the entire duration of the transfer.
    /// - `minor_loop_bytes` is a multiple of both `source_size` and `dest_size` bytes.
    /// - `major_loop_count` is at least 1.
    pub unsafe fn configure(&mut self, config: &TransferConfig) {
        let dma = dma_regs();
        let tcd = dma.tcd(CH as usize);
        // SAFETY for all TCD register writes below: Values come from the
        // caller-validated TransferConfig. Signed-to-unsigned casts (i16→u16,
        // i32→u32) preserve two's complement bit patterns as the hardware
        // expects. Field widths match K20 TCD register definitions.

        // Source address
        tcd.saddr().write(|w| unsafe { w.saddr().bits(config.source_addr) });

        // Source offset (signed i16, bit-pattern preserved by cast to u16)
        tcd.soff().write(|w| unsafe { w.soff().bits(config.source_offset as u16) });

        // Transfer attributes: SSIZE/DSIZE via safe enums, SMOD=0, DMOD from config
        tcd.attr().write(|w| {
            let w = unsafe { w.smod().bits(0).dmod().bits(config.dest_modulo) };
            let w = match config.source_size {
                TransferSize::Bits8 => w.ssize().bits8(),
                TransferSize::Bits16 => w.ssize().bits16(),
                TransferSize::Bits32 => w.ssize().bits32(),
                TransferSize::Burst16 => w.ssize().burst16(),
            };
            match config.dest_size {
                TransferSize::Bits8 => w.dsize().bits8(),
                TransferSize::Bits16 => w.dsize().bits16(),
                TransferSize::Bits32 => w.dsize().bits32(),
                TransferSize::Burst16 => w.dsize().burst16(),
            }
        });

        // Minor loop byte count (CR.EMLM=0, so use simple 32-bit NBYTES)
        tcd.nbytes_mlno().write(|w| unsafe { w.nbytes().bits(config.minor_loop_bytes) });

        // Source last address adjustment (signed i32 → u32)
        tcd.slast().write(|w| unsafe { w.slast().bits(config.source_last_adjust as u32) });

        // Destination address
        tcd.daddr().write(|w| unsafe { w.daddr().bits(config.dest_addr) });

        // Destination offset (signed i16 → u16)
        tcd.doff().write(|w| unsafe { w.doff().bits(config.dest_offset as u16) });

        // Current major iteration count (no channel linking)
        tcd.citer_elinkno().write(|w| {
            unsafe { w.citer().bits(config.major_loop_count) }
                .elink()._0()
        });

        // Destination last address adjustment (signed i32 → u32)
        tcd.dlastsga().write(|w| unsafe { w.dlastsga().bits(config.dest_last_adjust as u32) });

        // Control/status: optionally auto-disable request on major complete
        if config.auto_disable {
            tcd.csr().write(|w| w.dreq()._1());
        } else {
            tcd.csr().write(|w| w.dreq()._0());
        }

        // Beginning iteration count (must equal CITER when loading a new TCD)
        tcd.biter_elinkno().write(|w| {
            unsafe { w.biter().bits(config.major_loop_count) }
                .elink()._0()
        });
    }

    /// Configure the TCD with channel linking.
    ///
    /// Like [`configure`](DmaChannel::configure), but also sets up channel
    /// linking in the CITER/BITER and/or CSR registers.
    ///
    /// When minor loop linking is enabled, the maximum `major_loop_count`
    /// is reduced from 32767 to 511 (9-bit field).
    ///
    /// # Safety
    ///
    /// Same requirements as [`configure`](DmaChannel::configure).
    pub unsafe fn configure_linked(&mut self, config: &TransferConfig, link: ChannelLink) {
        match link {
            ChannelLink::None => {
                // Delegate to existing configure()
                self.configure(config);
                return;
            }
            _ => {}
        }

        let dma = dma_regs();
        let tcd = dma.tcd(CH as usize);

        // Common TCD fields (same as configure())
        tcd.saddr().write(|w| unsafe { w.saddr().bits(config.source_addr) });
        tcd.soff().write(|w| unsafe { w.soff().bits(config.source_offset as u16) });
        tcd.attr().write(|w| {
            let w = unsafe { w.smod().bits(0).dmod().bits(config.dest_modulo) };
            let w = match config.source_size {
                TransferSize::Bits8 => w.ssize().bits8(),
                TransferSize::Bits16 => w.ssize().bits16(),
                TransferSize::Bits32 => w.ssize().bits32(),
                TransferSize::Burst16 => w.ssize().burst16(),
            };
            match config.dest_size {
                TransferSize::Bits8 => w.dsize().bits8(),
                TransferSize::Bits16 => w.dsize().bits16(),
                TransferSize::Bits32 => w.dsize().bits32(),
                TransferSize::Burst16 => w.dsize().burst16(),
            }
        });
        tcd.nbytes_mlno().write(|w| unsafe { w.nbytes().bits(config.minor_loop_bytes) });
        tcd.slast().write(|w| unsafe { w.slast().bits(config.source_last_adjust as u32) });
        tcd.daddr().write(|w| unsafe { w.daddr().bits(config.dest_addr) });
        tcd.doff().write(|w| unsafe { w.doff().bits(config.dest_offset as u16) });
        tcd.dlastsga().write(|w| unsafe { w.dlastsga().bits(config.dest_last_adjust as u32) });

        // CITER/BITER and CSR depend on linking mode
        let minor_link = matches!(link, ChannelLink::MinorLoop { .. } | ChannelLink::Both { .. });
        let (major_link, major_ch) = match link {
            ChannelLink::MajorLoop { channel } => (true, channel),
            ChannelLink::Both { major_channel, .. } => (true, major_channel),
            _ => (false, 0),
        };

        if minor_link {
            let minor_ch = match link {
                ChannelLink::MinorLoop { channel } => channel,
                ChannelLink::Both { minor_channel, .. } => minor_channel,
                _ => unreachable!(),
            };
            // Use elinkyes variants: 9-bit count, 4-bit link channel, ELINK=1
            tcd.citer_elinkyes().write(|w| {
                unsafe { w.citer().bits(config.major_loop_count).linkch().bits(minor_ch) }
                    .elink()._1()
            });
            tcd.biter_elinkyes().write(|w| {
                unsafe { w.biter().bits(config.major_loop_count).linkch().bits(minor_ch) }
                    .elink()._1()
            });
        } else {
            // No minor linking — use elinkno variants
            tcd.citer_elinkno().write(|w| {
                unsafe { w.citer().bits(config.major_loop_count) }
                    .elink()._0()
            });
            tcd.biter_elinkno().write(|w| {
                unsafe { w.biter().bits(config.major_loop_count) }
                    .elink()._0()
            });
        }

        // CSR: optionally DREQ + optional major loop linking
        let dreq = config.auto_disable;
        if major_link {
            tcd.csr().write(|w| {
                let w = unsafe { w.majorlinkch().bits(major_ch) }
                    .majorelink()._1();
                if dreq { w.dreq()._1() } else { w.dreq()._0() }
            });
        } else if dreq {
            tcd.csr().write(|w| w.dreq()._1());
        } else {
            tcd.csr().write(|w| w.dreq()._0());
        }
    }

    /// Configure the TCD for scatter-gather operation.
    ///
    /// Copies the first TCD from a scatter-gather chain into the hardware
    /// TCD registers. When the major loop completes, the DMA engine
    /// automatically loads the next TCD from `dlastsga`.
    ///
    /// # Safety
    ///
    /// - All TCDs in the chain must be `'static` — the DMA engine reads
    ///   them asynchronously after major loop completion.
    /// - TCDs must be 32-byte aligned (enforced by `#[repr(align(32))]`).
    /// - Circular chains must have the last TCD's `dlastsga` pointing
    ///   back to the first TCD.
    /// - Source and destination addresses in each TCD must be valid and
    ///   properly aligned.
    pub unsafe fn configure_scatter_gather(&mut self, first_tcd: &'static ScatterGatherTcd) {
        let dma = dma_regs();
        let tcd = dma.tcd(CH as usize);

        tcd.saddr().write(|w| unsafe { w.saddr().bits(first_tcd.saddr) });
        tcd.soff().write(|w| unsafe { w.soff().bits(first_tcd.soff) });
        tcd.attr().write(|w| unsafe { w.bits(first_tcd.attr) });
        tcd.nbytes_mlno().write(|w| unsafe { w.nbytes().bits(first_tcd.nbytes) });
        tcd.slast().write(|w| unsafe { w.slast().bits(first_tcd.slast) });
        tcd.daddr().write(|w| unsafe { w.daddr().bits(first_tcd.daddr) });
        tcd.doff().write(|w| unsafe { w.doff().bits(first_tcd.doff) });
        tcd.citer_elinkno().write(|w| unsafe { w.bits(first_tcd.citer) });
        tcd.dlastsga().write(|w| unsafe { w.dlastsga().bits(first_tcd.dlastsga) });
        // Set ESG=1 in CSR, overriding whatever the TCD struct had
        tcd.csr().write(|w| w.esg()._1());
        tcd.biter_elinkno().write(|w| unsafe { w.bits(first_tcd.biter) });
    }

    // --- Transfer Control ---

    /// Enable hardware DMA requests for this channel (set ERQ bit via SERQ).
    pub fn enable_request(&mut self) {
        // SAFETY: CH is a const generic validated at channel construction.
        // This same rationale applies to all similar DMA command register
        // writes (cerq, ssrt, cdne, cint, cerr, seei, ceei) in this impl.
        dma_regs().serq().write(|w| unsafe { w.serq().bits(CH) });
    }

    /// Disable hardware DMA requests for this channel (clear ERQ bit via CERQ).
    pub fn disable_request(&mut self) {
        dma_regs().cerq().write(|w| unsafe { w.cerq().bits(CH) });
    }

    /// Trigger a single transfer by software (set START bit via SSRT).
    pub fn start(&mut self) {
        dma_regs().ssrt().write(|w| unsafe { w.ssrt().bits(CH) });
    }

    // --- Status ---

    /// Check if the major loop is complete (CSR.DONE flag).
    pub fn is_complete(&self) -> bool {
        dma_regs().tcd(CH as usize).csr().read().done().bit_is_set()
    }

    /// Check if this channel has an error (ERR register bit).
    pub fn has_error(&self) -> bool {
        dma_regs().err().read().bits() & (1 << CH) != 0
    }

    /// Check if this channel is actively transferring (CSR.ACTIVE).
    pub fn is_active(&self) -> bool {
        dma_regs().tcd(CH as usize).csr().read().active().bit_is_set()
    }

    /// Read error details from the ES register.
    ///
    /// Returns the error type if a valid error exists for this channel,
    /// `None` otherwise. The ES register is global and only records the
    /// most recent error.
    pub fn error_status(&self) -> Option<DmaError> {
        let es = dma_regs().es().read();
        if !es.vld().is_1() {
            return None;
        }
        if es.errchn().bits() != CH {
            return None;
        }

        if es.sae().is_1() {
            Some(DmaError::SourceAddressError)
        } else if es.soe().is_1() {
            Some(DmaError::SourceOffsetError)
        } else if es.dae().is_1() {
            Some(DmaError::DestAddressError)
        } else if es.doe().is_1() {
            Some(DmaError::DestOffsetError)
        } else if es.nce().is_1() {
            Some(DmaError::NbytesConfigError)
        } else if es.sge().is_1() {
            Some(DmaError::ScatterGatherError)
        } else if es.sbe().is_1() {
            Some(DmaError::SourceBusError)
        } else if es.dbe().is_1() {
            Some(DmaError::DestBusError)
        } else if es.ecx().is_1() {
            Some(DmaError::Cancelled)
        } else {
            None
        }
    }

    // --- Flag Management ---

    /// Clear the DONE flag for this channel (via CDNE).
    pub fn clear_done(&mut self) {
        dma_regs().cdne().write(|w| unsafe { w.cdne().bits(CH) });
    }

    /// Clear the interrupt request flag for this channel (via CINT).
    pub fn clear_interrupt(&mut self) {
        dma_regs().cint().write(|w| unsafe { w.cint().bits(CH) });
    }

    /// Clear the error flag for this channel (via CERR).
    pub fn clear_error(&mut self) {
        dma_regs().cerr().write(|w| unsafe { w.cerr().bits(CH) });
    }

    // --- Interrupts ---

    /// Enable interrupt on major loop completion (CSR.INTMAJOR).
    pub fn enable_interrupt(&mut self) {
        dma_regs().tcd(CH as usize).csr().modify(|_, w| w.intmajor()._1());
    }

    /// Disable interrupt on major loop completion.
    pub fn disable_interrupt(&mut self) {
        dma_regs().tcd(CH as usize).csr().modify(|_, w| w.intmajor()._0());
    }

    /// Enable error interrupt for this channel (set EEI bit via SEEI).
    pub fn enable_error_interrupt(&mut self) {
        dma_regs().seei().write(|w| unsafe { w.seei().bits(CH) });
    }

    /// Disable error interrupt for this channel (clear EEI bit via CEEI).
    pub fn disable_error_interrupt(&mut self) {
        dma_regs().ceei().write(|w| unsafe { w.ceei().bits(CH) });
    }

    // --- TCD Readback ---

    /// Read the current destination address from the TCD.
    ///
    /// For circular DMA transfers (dest_modulo > 0), this returns the
    /// address where the DMA engine will write next, which advances
    /// independently of the CPU.
    pub fn current_dest_addr(&self) -> u32 {
        dma_regs().tcd(CH as usize).daddr().read().daddr().bits()
    }

    // --- Convenience Methods ---

    /// Configure a memory-to-memory copy of `len` bytes.
    ///
    /// Uses 32-bit transfers when both addresses and length are 4-byte aligned,
    /// otherwise falls back to 8-bit transfers. The entire copy runs as a
    /// single major loop iteration.
    ///
    /// After calling this, use [`start`](DmaChannel::start) to trigger the
    /// transfer via software.
    ///
    /// # Safety
    ///
    /// Caller must ensure source and destination memory regions are valid,
    /// do not overlap, and remain valid until the transfer completes.
    pub unsafe fn configure_memcpy(&mut self, src: *const u8, dst: *mut u8, len: u32) {
        let (size, offset) = if (src as u32) % 4 == 0 && (dst as u32) % 4 == 0 && len % 4 == 0 {
            (TransferSize::Bits32, 4i16)
        } else {
            (TransferSize::Bits8, 1i16)
        };

        self.configure(&TransferConfig {
            source_addr: src as u32,
            dest_addr: dst as u32,
            source_size: size,
            dest_size: size,
            source_offset: offset,
            dest_offset: offset,
            minor_loop_bytes: len,
            major_loop_count: 1,
            source_last_adjust: -(len as i32),
            dest_last_adjust: -(len as i32),
            dest_modulo: 0,
            auto_disable: true,
        });
    }

    /// Configure a peripheral-to-memory transfer.
    ///
    /// Reads `count` values from a fixed peripheral register address into a
    /// buffer. Each DMA activation reads one value; the major loop runs
    /// `count` times. The destination pointer resets after completion.
    ///
    /// After calling this, use [`set_source`](DmaChannel::set_source) and
    /// [`enable_request`](DmaChannel::enable_request) to start hardware-triggered
    /// transfers.
    ///
    /// # Safety
    ///
    /// Caller must ensure the peripheral address is valid and the buffer is
    /// large enough for `count * transfer_size.bytes()` bytes.
    pub unsafe fn configure_peripheral_read(
        &mut self,
        periph_addr: u32,
        buffer: *mut u8,
        transfer_size: TransferSize,
        count: u16,
    ) {
        let ts_bytes = transfer_size.bytes();
        self.configure(&TransferConfig {
            source_addr: periph_addr,
            dest_addr: buffer as u32,
            source_size: transfer_size,
            dest_size: transfer_size,
            source_offset: 0,
            // TransferSize::bytes() returns at most 16, which fits in i16.
            dest_offset: ts_bytes as i16,
            minor_loop_bytes: ts_bytes,
            major_loop_count: count,
            source_last_adjust: 0,
            dest_last_adjust: -(count as i32 * ts_bytes as i32),
            dest_modulo: 0,
            auto_disable: true,
        });
    }

    /// Configure a memory-to-peripheral transfer.
    ///
    /// Writes `count` values from a buffer to a fixed peripheral register
    /// address. Each DMA activation writes one value; the major loop runs
    /// `count` times. The source pointer resets after completion.
    ///
    /// After calling this, use [`set_source`](DmaChannel::set_source) and
    /// [`enable_request`](DmaChannel::enable_request) to start hardware-triggered
    /// transfers.
    ///
    /// # Safety
    ///
    /// Caller must ensure the peripheral address is valid and the buffer
    /// contains at least `count * transfer_size.bytes()` bytes.
    pub unsafe fn configure_peripheral_write(
        &mut self,
        buffer: *const u8,
        periph_addr: u32,
        transfer_size: TransferSize,
        count: u16,
    ) {
        let ts_bytes = transfer_size.bytes();
        self.configure(&TransferConfig {
            source_addr: buffer as u32,
            dest_addr: periph_addr,
            source_size: transfer_size,
            dest_size: transfer_size,
            source_offset: ts_bytes as i16,
            dest_offset: 0,
            minor_loop_bytes: ts_bytes,
            major_loop_count: count,
            source_last_adjust: -(count as i32 * ts_bytes as i32),
            dest_last_adjust: 0,
            dest_modulo: 0,
            auto_disable: true,
        });
    }
}

// ----- DMA Transfer Handle -----

/// A safe handle for an in-progress DMA transfer.
///
/// Ties the lifetime of the buffer to the transfer, preventing
/// the buffer from being freed or modified while DMA is active.
///
/// **Drop behavior:** Dropping this value aborts the in-progress DMA
/// transfer by disabling hardware requests and clearing status flags.
/// If you want the transfer to complete, call [`wait()`](DmaTransfer::wait)
/// instead of dropping.
pub struct DmaTransfer<'a, const CH: u8> {
    pub(crate) channel: &'a mut DmaChannel<CH>,
}

impl<'a, const CH: u8> DmaTransfer<'a, CH> {
    /// Check if the transfer has completed.
    pub fn is_complete(&self) -> bool {
        self.channel.is_complete()
    }

    /// Check if the transfer has errored.
    pub fn has_error(&self) -> bool {
        self.channel.has_error()
    }

    /// Block until the transfer completes or errors.
    ///
    /// Consumes the transfer handle without running its Drop impl (which
    /// would abort the transfer). On success, returns the DMA channel
    /// for reuse. On error, the channel is cleaned up before returning.
    pub fn wait(self) -> Result<&'a mut DmaChannel<CH>, DmaError> {
        loop {
            if self.channel.has_error() {
                let err = self.channel.error_status().unwrap_or(DmaError::Cancelled);
                self.channel.clear_error();
                self.channel.clear_done();
                self.channel.disable_request();
                core::mem::forget(self);
                return Err(err);
            }
            if self.channel.is_complete() {
                self.channel.clear_done();
                self.channel.disable_request();
                // SAFETY: We need to return the &'a mut DmaChannel while
                // preventing Drop from running (it would abort the completed
                // transfer). The raw pointer preserves the original 'a lifetime
                // from self.channel, and forget() suppresses the Drop impl.
                let ch_ptr = self.channel as *const DmaChannel<CH> as *mut DmaChannel<CH>;
                core::mem::forget(self);
                return Ok(unsafe { &mut *ch_ptr });
            }
        }
    }
}

impl<'a, const CH: u8> Drop for DmaTransfer<'a, CH> {
    fn drop(&mut self) {
        // Abort the transfer on drop
        self.channel.disable_request();
        self.channel.clear_done();
        self.channel.clear_error();
    }
}

// ----- DMA Channels -----

/// All DMA channels returned by [`DmaExt::split`].
pub struct DmaChannels {
    pub ch0: DmaChannel<0>,
    pub ch1: DmaChannel<1>,
    pub ch2: DmaChannel<2>,
    pub ch3: DmaChannel<3>,
    #[cfg(feature = "mk20d7")]
    pub ch4: DmaChannel<4>,
    #[cfg(feature = "mk20d7")]
    pub ch5: DmaChannel<5>,
    #[cfg(feature = "mk20d7")]
    pub ch6: DmaChannel<6>,
    #[cfg(feature = "mk20d7")]
    pub ch7: DmaChannel<7>,
    #[cfg(feature = "mk20d7")]
    pub ch8: DmaChannel<8>,
    #[cfg(feature = "mk20d7")]
    pub ch9: DmaChannel<9>,
    #[cfg(feature = "mk20d7")]
    pub ch10: DmaChannel<10>,
    #[cfg(feature = "mk20d7")]
    pub ch11: DmaChannel<11>,
    #[cfg(feature = "mk20d7")]
    pub ch12: DmaChannel<12>,
    #[cfg(feature = "mk20d7")]
    pub ch13: DmaChannel<13>,
    #[cfg(feature = "mk20d7")]
    pub ch14: DmaChannel<14>,
    #[cfg(feature = "mk20d7")]
    pub ch15: DmaChannel<15>,
}

// ----- Extension Trait -----

/// Extension trait for the DMA peripheral.
///
/// Consumes the DMA and DMAMUX peripherals, enables their clocks,
/// configures default channel priorities, and returns individual channel handles.
pub trait DmaExt: Sized {
    /// Consume the DMA and DMAMUX peripherals and return individual channels.
    ///
    /// Enables the DMA and DMAMUX clock gates, configures the DMA controller
    /// to stall in debug mode with fixed-priority arbitration, sets default
    /// channel priorities (channel N = priority N), and disables all DMAMUX
    /// channels.
    fn split(self, dmamux: pac::Dmamux, sim: &pac::Sim) -> DmaChannels;
}

impl DmaExt for pac::Dma {
    fn split(self, _dmamux: pac::Dmamux, sim: &pac::Sim) -> DmaChannels {
        // Enable DMAMUX clock gate
        sim.scgc6().modify(|_, w| w.dmamux().enabled());
        // Enable DMA clock gate
        sim.scgc7().modify(|_, w| w.dma().enabled());

        let dma = dma_regs();
        let dmamux = dmamux_regs();

        // Configure CR: stall in debug mode, fixed priority, EMLM=0
        dma.cr().write(|w| w.edbg()._1());

        // Set default channel priorities: channel N gets priority N, preemptable
        for ch in 0..NUM_CHANNELS as u8 {
            // SAFETY: ch iterates 0..NUM_CHANNELS, fitting in the 4-bit
            // chpri field. dchpri_index handles the byte-swap addressing.
            dma.dchpri(dchpri_index(ch)).write(|w| {
                unsafe { w.chpri().bits(ch) }
                    .ecp()._1()
            });
        }

        // Disable all DMAMUX channels
        for ch in 0..NUM_CHANNELS {
            dmamux.chcfg(ch).write(|w| w);
        }

        DmaChannels {
            ch0: DmaChannel,
            ch1: DmaChannel,
            ch2: DmaChannel,
            ch3: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch4: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch5: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch6: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch7: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch8: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch9: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch10: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch11: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch12: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch13: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch14: DmaChannel,
            #[cfg(feature = "mk20d7")]
            ch15: DmaChannel,
        }
    }
}

// ----- Async support -----

#[cfg(feature = "async")]
mod async_impl {
    //! Async support assumes a single-executor, single-core environment.
    //! Global wakers are used per channel — only one task may await a
    //! given channel's completion at a time.

    use super::*;
    use embassy_sync::waitqueue::AtomicWaker;

    #[cfg(feature = "mk20d5")]
    static DMA_WAKERS: [AtomicWaker; 4] = [
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
    ];

    #[cfg(feature = "mk20d7")]
    static DMA_WAKERS: [AtomicWaker; 16] = [
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
        AtomicWaker::new(),
    ];

    /// Call from a DMA channel interrupt handler. Clears the interrupt flag
    /// and wakes the corresponding waker.
    ///
    /// Silently ignores channel numbers >= NUM_CHANNELS (prevents panic on
    /// misrouted interrupts).
    pub fn on_dma_interrupt(ch: u8) {
        if (ch as usize) >= DMA_WAKERS.len() {
            return;
        }
        let dma = dma_regs();
        // SAFETY: ch is validated above to be within range. The cint field
        // accepts a channel number to clear a single channel's interrupt flag.
        dma.cint().write(|w| unsafe { w.cint().bits(ch) });
        DMA_WAKERS[ch as usize].wake();
    }

    /// Call from the `DMA_CH0` interrupt handler.
    pub fn on_dma0_interrupt() {
        on_dma_interrupt(0);
    }

    /// Call from the `DMA_CH1` interrupt handler.
    pub fn on_dma1_interrupt() {
        on_dma_interrupt(1);
    }

    /// Call from the `DMA_CH2` interrupt handler.
    pub fn on_dma2_interrupt() {
        on_dma_interrupt(2);
    }

    /// Call from the `DMA_CH3` interrupt handler.
    pub fn on_dma3_interrupt() {
        on_dma_interrupt(3);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH4` interrupt handler.
    pub fn on_dma4_interrupt() {
        on_dma_interrupt(4);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH5` interrupt handler.
    pub fn on_dma5_interrupt() {
        on_dma_interrupt(5);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH6` interrupt handler.
    pub fn on_dma6_interrupt() {
        on_dma_interrupt(6);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH7` interrupt handler.
    pub fn on_dma7_interrupt() {
        on_dma_interrupt(7);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH8` interrupt handler.
    pub fn on_dma8_interrupt() {
        on_dma_interrupt(8);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH9` interrupt handler.
    pub fn on_dma9_interrupt() {
        on_dma_interrupt(9);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH10` interrupt handler.
    pub fn on_dma10_interrupt() {
        on_dma_interrupt(10);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH11` interrupt handler.
    pub fn on_dma11_interrupt() {
        on_dma_interrupt(11);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH12` interrupt handler.
    pub fn on_dma12_interrupt() {
        on_dma_interrupt(12);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH13` interrupt handler.
    pub fn on_dma13_interrupt() {
        on_dma_interrupt(13);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH14` interrupt handler.
    pub fn on_dma14_interrupt() {
        on_dma_interrupt(14);
    }

    #[cfg(feature = "mk20d7")]
    /// Call from the `DMA_CH15` interrupt handler.
    pub fn on_dma15_interrupt() {
        on_dma_interrupt(15);
    }

    impl<const CH: u8> DmaChannel<CH> {
        /// Await DMA transfer completion.
        ///
        /// Enables the major-loop-complete interrupt, waits for DONE or ERROR,
        /// then disables the interrupt. The channel must already be configured
        /// and started before calling this.
        ///
        /// The caller must wire the corresponding `DMA_CH{N}` interrupt to
        /// [`on_dmaN_interrupt()`] and unmask it in the NVIC.
        pub async fn wait_complete(&mut self) -> Result<(), DmaError> {
            self.enable_interrupt();
            let result = core::future::poll_fn(|cx| {
                DMA_WAKERS[CH as usize].register(cx.waker());
                if self.is_complete() {
                    core::task::Poll::Ready(Ok(()))
                } else if self.has_error() {
                    core::task::Poll::Ready(Err(
                        self.error_status().unwrap_or(DmaError::Cancelled),
                    ))
                } else {
                    core::task::Poll::Pending
                }
            })
            .await;
            self.disable_interrupt();
            self.clear_done();
            result
        }
    }
}

#[cfg(feature = "async")]
pub use async_impl::{
    on_dma0_interrupt, on_dma1_interrupt, on_dma2_interrupt, on_dma3_interrupt, on_dma_interrupt,
};
#[cfg(all(feature = "async", feature = "mk20d7"))]
pub use async_impl::{
    on_dma10_interrupt, on_dma11_interrupt, on_dma12_interrupt, on_dma13_interrupt,
    on_dma14_interrupt, on_dma15_interrupt, on_dma4_interrupt, on_dma5_interrupt,
    on_dma6_interrupt, on_dma7_interrupt, on_dma8_interrupt, on_dma9_interrupt,
};
