//! USB device driver for the Kinetis USB-FS controller.
//!
//! Implements the [`usb_device::bus::UsbBus`] trait using the hardware BDT
//! (Buffer Descriptor Table) with ping-pong buffering. Supports 16 endpoints,
//! each bidirectional with EVEN/ODD banks.
//!
//! Both MK20D5 (Teensy 3.0) and MK20D7 (Teensy 3.1/3.2) have identical USB0
//! peripherals — no `#[cfg]` gating needed.
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//! use usb_device::prelude::*;
//!
//! let dp = pac::Peripherals::take().unwrap();
//! dp.WDOG.disable();
//! let clocks = dp.MCG.constrain().freeze(dp.OSC, &dp.SIM);
//!
//! let usb_bus = dp.USB0.usb_bus(&dp.SIM);
//! let usb_bus_alloc = UsbBusAllocator::new(usb_bus);
//!
//! let mut usb_dev = UsbDeviceBuilder::new(&usb_bus_alloc, UsbVidPid(0x16c0, 0x27dd))
//!     .build();
//!
//! loop {
//!     usb_dev.poll(&mut []);
//! }
//! ```

use core::cell::UnsafeCell;

use crate::pac;
use usb_device::bus::{PollResult, UsbBus as UsbBusTrait};
use usb_device::endpoint::{EndpointAddress, EndpointType};
use usb_device::{UsbDirection, UsbError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NUM_ENDPOINTS: usize = 16;
const EP_BUF_SIZE: usize = 64;

// BDT descriptor bits (CPU → USB direction)
const BD_OWN: u32 = 1 << 7;
const BD_DATA1: u32 = 1 << 6;
const BD_DTS: u32 = 1 << 3;
const BD_STALL: u32 = 1 << 2;

// USB PID tokens (from completed BD, bits [5:2])
const PID_SETUP: u8 = 0x0D;

// ISTAT bit positions
const ISTAT_USBRST: u8 = 1 << 0;
const ISTAT_ERROR: u8 = 1 << 1;
const ISTAT_TOKDNE: u8 = 1 << 3;
const ISTAT_SLEEP: u8 = 1 << 4;
const ISTAT_RESUME: u8 = 1 << 5;
const ISTAT_STALL: u8 = 1 << 7;

// ---------------------------------------------------------------------------
// USB register access (via raw pointer, like DMA pattern)
// ---------------------------------------------------------------------------

fn usb_regs() -> &'static pac::usb0::RegisterBlock {
    // SAFETY: PTR is a valid pointer to the USB0 register block, which is
    // memory-mapped I/O that exists for the lifetime of the program.
    unsafe { &*pac::Usb0::PTR }
}

fn sim_regs() -> &'static pac::sim::RegisterBlock {
    // SAFETY: PTR is a valid pointer to the SIM register block.
    unsafe { &*pac::Sim::PTR }
}

// ---------------------------------------------------------------------------
// BDT (Buffer Descriptor Table)
// ---------------------------------------------------------------------------

/// A single buffer descriptor entry (8 bytes).
#[derive(Clone, Copy)]
#[repr(C)]
struct BufferDescriptor {
    desc: u32,
    addr: u32,
}

/// Full BDT: 16 endpoints × 2 directions × 2 ping-pong = 64 entries.
/// Must be 512-byte aligned.
#[repr(C, align(512))]
struct Bdt {
    entries: [BufferDescriptor; NUM_ENDPOINTS * 4],
}

static mut BDT: Bdt = Bdt {
    entries: [BufferDescriptor { desc: 0, addr: 0 }; NUM_ENDPOINTS * 4],
};

// ---------------------------------------------------------------------------
// Endpoint buffer pool
// ---------------------------------------------------------------------------

/// Static buffer pool — 64 buffers of 64 bytes each, 4-byte aligned.
#[repr(C, align(4))]
struct EpBufPool {
    bufs: [[u8; EP_BUF_SIZE]; NUM_ENDPOINTS * 4],
}

static mut EP_BUFS: EpBufPool = EpBufPool {
    bufs: [[0; EP_BUF_SIZE]; NUM_ENDPOINTS * 4],
};

/// BDT index: ep * 4 + (tx ? 2 : 0) + (odd ? 1 : 0)
const fn bdt_index(ep: usize, tx: bool, odd: bool) -> usize {
    ep * 4 + if tx { 2 } else { 0 } + if odd { 1 } else { 0 }
}

/// Extract the byte count from a completed BDT descriptor (bits [25:16]).
fn bd_byte_count(desc: u32) -> u16 {
    ((desc >> 16) & 0x3FF) as u16
}

/// Extract the PID token from a completed BDT descriptor (bits [5:2]).
fn bd_pid(desc: u32) -> u8 {
    ((desc >> 2) & 0xF) as u8
}

fn make_bd_desc(byte_count: u16, data1: bool, own: bool, dts: bool, stall: bool) -> u32 {
    let mut desc = (byte_count as u32 & 0x3FF) << 16;
    if own {
        desc |= BD_OWN;
    }
    if data1 {
        desc |= BD_DATA1;
    }
    if dts {
        desc |= BD_DTS;
    }
    if stall {
        desc |= BD_STALL;
    }
    desc
}

/// Read a BDT descriptor using a volatile read.
///
/// The USB hardware controller modifies BDT entries asynchronously when
/// it completes a transfer (clearing OWN, writing byte count and PID).
/// Volatile reads ensure we see the hardware's latest write.
fn bdt_read_desc(idx: usize) -> u32 {
    // SAFETY: idx is always computed via bdt_index() which is bounded by
    // NUM_ENDPOINTS * 4. The BDT static is only accessed through these
    // helpers and the reset path (which runs with USB disabled).
    unsafe {
        let bdt_ptr = &raw const BDT;
        core::ptr::read_volatile(&raw const (*bdt_ptr).entries[idx].desc)
    }
}

/// Write a BDT entry (address + descriptor) using volatile writes.
///
/// The address must be written before the descriptor because the USB
/// controller reads the address as soon as OWN is set in the descriptor.
/// A compiler fence ensures the ordering is preserved.
fn bdt_write(idx: usize, desc: u32, addr: u32) {
    // SAFETY: idx is bounded by NUM_ENDPOINTS * 4 (see bdt_index()).
    // Volatile writes ensure the hardware sees our updates. The Release
    // fence guarantees addr is visible before desc (which contains OWN).
    unsafe {
        let bdt_ptr = &raw mut BDT;
        core::ptr::write_volatile(&raw mut (*bdt_ptr).entries[idx].addr, addr);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
        core::ptr::write_volatile(&raw mut (*bdt_ptr).entries[idx].desc, desc);
    }
}

// ---------------------------------------------------------------------------
// Endpoint state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct EndpointState {
    ep_type: Option<EndpointType>,
    max_packet_size: u16,
    stalled_in: bool,
    stalled_out: bool,
    // DATA0/DATA1 toggle tracking
    tx_data_toggle: bool,
    rx_data_toggle: bool,
    // Ping-pong bank tracking
    tx_odd: bool,
    rx_odd: bool,
    // Whether there is unread data in the RX buffer
    rx_ready: bool,
    // Which bank the completed RX data is in
    rx_complete_odd: bool,
    // Whether this was a SETUP packet
    rx_setup: bool,
}

impl EndpointState {
    const fn new() -> Self {
        Self {
            ep_type: None,
            max_packet_size: 0,
            stalled_in: false,
            stalled_out: false,
            tx_data_toggle: false,
            rx_data_toggle: false,
            tx_odd: false,
            rx_odd: false,
            rx_ready: false,
            rx_complete_odd: false,
            rx_setup: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Inner mutable state
// ---------------------------------------------------------------------------

struct Inner {
    endpoints: [EndpointState; NUM_ENDPOINTS],
    ep_alloc_mask: u16,
}

impl Inner {
    const fn new() -> Self {
        Self {
            endpoints: [EndpointState::new(); NUM_ENDPOINTS],
            ep_alloc_mask: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// UsbBus
// ---------------------------------------------------------------------------

/// USB bus driver implementing `usb_device::bus::UsbBus`.
///
/// Created by calling [`UsbBusExt::usb_bus`] on the USB0 PAC peripheral.
/// Interior mutability is used because the `UsbBus` trait requires `&self`
/// for most methods after the allocation phase.
pub struct UsbBus {
    inner: UnsafeCell<Inner>,
}

// SAFETY: Single-core Cortex-M4 with no preemptive threading. The USB bus
// is accessed from a single context (the USB polling loop). The UnsafeCell
// interior mutability is safe because there is no concurrent access.
unsafe impl Send for UsbBus {}
unsafe impl Sync for UsbBus {}

impl UsbBus {
    fn new() -> Self {
        Self {
            inner: UnsafeCell::new(Inner::new()),
        }
    }

    fn inner(&self) -> &Inner {
        // SAFETY: Single-core Cortex-M4, USB accessed from one context.
        // UsbBusTrait methods take &self but are only called from the USB
        // polling loop. No concurrent access occurs.
        unsafe { &*self.inner.get() }
    }

    fn inner_mut(&self) -> &mut Inner {
        // SAFETY: Same invariant as inner(). Single-core, single-context USB access.
        unsafe { &mut *self.inner.get() }
    }

    /// Arm an RX buffer descriptor for the given endpoint.
    fn arm_rx(&self, ep: usize, odd: bool) {
        let state = &self.inner().endpoints[ep];
        let idx = bdt_index(ep, false, odd);
        let desc = make_bd_desc(
            state.max_packet_size,
            state.rx_data_toggle,
            true, // OWN = USB
            true, // DTS
            false,
        );
        // SAFETY: EP_BUFS is a static buffer pool that exists for the program's
        // lifetime. The index is computed from a valid endpoint number.
        let buf_addr = unsafe { EP_BUFS.bufs[idx].as_ptr() as u32 };
        bdt_write(idx, desc, buf_addr);
    }

    /// Configure endpoint 0 for control transfers after reset.
    fn configure_ep0(&self) {
        let inner = self.inner_mut();
        let ep0 = &mut inner.endpoints[0];
        ep0.tx_data_toggle = false;
        ep0.rx_data_toggle = false;
        ep0.tx_odd = false;
        ep0.rx_odd = false;
        ep0.rx_ready = false;
        ep0.rx_setup = false;
        ep0.stalled_in = false;
        ep0.stalled_out = false;

        // Arm RX EVEN for EP0
        self.arm_rx(0, false);

        let usb = usb_regs();
        // EP0: handshake, TX, RX enabled (control endpoint)
        usb.endpt(0).write(|w| {
            w.ephshk().set_bit()
             .eptxen().set_bit()
             .eprxen().set_bit()
        });
    }
}

impl UsbBusTrait for UsbBus {
    // The Kinetis USB-FS sets the address immediately, so we need
    // set_device_address called before the status stage completes.
    const QUIRK_SET_ADDRESS_BEFORE_STATUS: bool = false;

    // --- Endpoint Allocation ---

    fn alloc_ep(
        &mut self,
        ep_dir: UsbDirection,
        ep_addr: Option<EndpointAddress>,
        ep_type: EndpointType,
        max_packet_size: u16,
        _interval: u8,
    ) -> usb_device::Result<EndpointAddress> {
        let inner = self.inner.get_mut();

        let ep_index = match ep_addr {
            Some(addr) => {
                let idx = addr.index();
                if idx >= NUM_ENDPOINTS {
                    return Err(UsbError::InvalidEndpoint);
                }
                // If the endpoint is already allocated, it must be the same type
                // (allows bidirectional allocation on the same endpoint number)
                if inner.ep_alloc_mask & (1 << idx) != 0 {
                    if let Some(existing_type) = inner.endpoints[idx].ep_type {
                        if existing_type != ep_type {
                            return Err(UsbError::InvalidEndpoint);
                        }
                    }
                }
                idx
            }
            None => {
                // Find first free endpoint (skip EP0 for non-control)
                let start = if ep_type == EndpointType::Control { 0 } else { 1 };
                let mut found = None;
                for i in start..NUM_ENDPOINTS {
                    if inner.ep_alloc_mask & (1 << i) == 0 {
                        found = Some(i);
                        break;
                    }
                }
                found.ok_or(UsbError::EndpointOverflow)?
            }
        };

        if max_packet_size > EP_BUF_SIZE as u16 {
            return Err(UsbError::EndpointMemoryOverflow);
        }

        inner.ep_alloc_mask |= 1 << ep_index;
        inner.endpoints[ep_index].ep_type = Some(ep_type);
        inner.endpoints[ep_index].max_packet_size = max_packet_size;

        Ok(EndpointAddress::from_parts(ep_index, ep_dir))
    }

    // --- Bus Lifecycle ---

    fn enable(&mut self) {
        let usb = usb_regs();
        let sim = sim_regs();

        // --- Configure USB 48 MHz clock source ---

        // Select PLL as source for USB clock
        sim.sopt2().modify(|_, w| {
            w.pllfllsel().pll()
             .usbsrc()._1()
        });

        // Configure USB clock divider: USB_CLK = PLL × (USBFRAC+1) / (USBDIV+1)
        #[cfg(feature = "mk20d7")]
        {
            // SAFETY: USBDIV is a 3-bit field; value 2 fits.
            // 72 MHz PLL: 72 × 2/3 = 48 MHz (USBFRAC=1, USBDIV=2)
            sim.clkdiv2().write(|w| unsafe {
                w.usbfrac().set_bit()
                 .usbdiv().bits(2)
            });
        }
        #[cfg(feature = "mk20d5")]
        {
            // SAFETY: USBDIV is a 3-bit field; value 0 fits.
            // 48 MHz PLL: 48 × 1/1 = 48 MHz (USBFRAC=0, USBDIV=0)
            sim.clkdiv2().write(|w| unsafe {
                w.usbdiv().bits(0)
            });
        }

        // Enable USB clock gate
        sim.scgc4().modify(|_, w| w.usbotg().enabled());

        // --- Initialize USB peripheral ---

        // SAFETY: bdtba fields accept the respective address bits. The BDT
        // static is 512-byte aligned (repr(C, align(512))).
        // Set BDT base address
        let bdt_addr = &raw const BDT as u32;
        usb.bdtpage1().write(|w| unsafe { w.bdtba().bits(((bdt_addr >> 9) & 0x7F) as u8) });
        usb.bdtpage2().write(|w| unsafe { w.bdtba().bits((bdt_addr >> 16) as u8) });
        usb.bdtpage3().write(|w| unsafe { w.bdtba().bits((bdt_addr >> 24) as u8) });

        // SAFETY: Writing 0xFF to w1c status registers clears all flags.
        // Clear all interrupt flags (w1c)
        usb.istat().write(|w| unsafe { w.bits(0xFF) });
        usb.errstat().write(|w| unsafe { w.bits(0xFF) });

        // Enable all error sources
        usb.erren().write(|w| unsafe { w.bits(0xFF) });

        // Enable interrupts: reset, token done, sleep, resume, stall, error
        usb.inten().write(|w| {
            w.usbrsten()._1()
             .erroren()._1()
             .tokdneen()._1()
             .sleepen()._1()
             .resumeen()._1()
             .stallen()._1()
        });

        // Disable weak pulldowns, not in suspend
        usb.usbctrl().write(|w| w.pde()._0().susp()._0());

        // Configure EP0
        self.configure_ep0();

        // Enable USB module (non-OTG mode: D+ pullup is automatic)
        usb.ctl().write(|w| w.usbensofen()._1());

        // Use non-OTG mode: OTGEN=0 means D+ pullup is controlled by USBENSOFEN
        usb.otgctl().write(|w| w.otgen()._0());
    }

    fn reset(&self) {
        let usb = usb_regs();
        let inner = self.inner_mut();

        // SAFETY: addr is a 7-bit field; masked to fit.
        // Clear USB address
        usb.addr().write(|w| unsafe { w.addr().bits(0) });

        // Clear all BDT entries with volatile writes — USB hardware may
        // still be reading these until ODDRST completes below.
        for i in 0..NUM_ENDPOINTS * 4 {
            bdt_write(i, 0, 0);
        }

        // Reset ODD toggle
        usb.ctl().modify(|_, w| w.oddrst().set_bit());
        // Immediately clear ODDRST
        usb.ctl().modify(|_, w| w.oddrst().clear_bit());

        // Reset all endpoint state
        for i in 0..NUM_ENDPOINTS {
            inner.endpoints[i].tx_data_toggle = false;
            inner.endpoints[i].rx_data_toggle = false;
            inner.endpoints[i].tx_odd = false;
            inner.endpoints[i].rx_odd = false;
            inner.endpoints[i].rx_ready = false;
            inner.endpoints[i].rx_setup = false;
            inner.endpoints[i].stalled_in = false;
            inner.endpoints[i].stalled_out = false;
        }

        // Disable all endpoints except EP0
        for i in 1..NUM_ENDPOINTS {
            usb.endpt(i).write(|w| w);
        }

        // Re-configure EP0
        self.configure_ep0();

        // Configure allocated non-zero endpoints
        for i in 1..NUM_ENDPOINTS {
            if inner.ep_alloc_mask & (1 << i) == 0 {
                continue;
            }
            let ep = &inner.endpoints[i];
            let ep_type = match ep.ep_type {
                Some(t) => t,
                None => continue,
            };

            let is_control = ep_type == EndpointType::Control;

            // Arm RX buffer
            self.arm_rx(i, false);

            usb.endpt(i).write(|w| {
                let w = w.ephshk().set_bit()
                          .eptxen().set_bit()
                          .eprxen().set_bit();
                if !is_control {
                    w.epctldis().set_bit()
                } else {
                    w
                }
            });
        }

        // Clear all pending interrupts
        usb.istat().write(|w| unsafe { w.bits(0xFF) });
        usb.errstat().write(|w| unsafe { w.bits(0xFF) });
    }

    fn set_device_address(&self, addr: u8) {
        // SAFETY: addr is a 7-bit field; masked to fit.
        usb_regs().addr().write(|w| unsafe { w.addr().bits(addr & 0x7F) });
    }

    // --- Data Transfer ---

    fn write(&self, ep_addr: EndpointAddress, buf: &[u8]) -> usb_device::Result<usize> {
        let ep = ep_addr.index();
        if ep >= NUM_ENDPOINTS {
            return Err(UsbError::InvalidEndpoint);
        }

        let inner = self.inner_mut();
        let state = &mut inner.endpoints[ep];

        if state.ep_type.is_none() {
            return Err(UsbError::InvalidEndpoint);
        }

        if state.stalled_in {
            return Err(UsbError::InvalidState);
        }

        let len = buf.len().min(state.max_packet_size as usize);
        let odd = state.tx_odd;
        let idx = bdt_index(ep, true, odd);

        // Check that the buffer is not owned by USB
        let bd_desc = bdt_read_desc(idx);
        if bd_desc & BD_OWN != 0 {
            return Err(UsbError::WouldBlock);
        }

        // SAFETY: EP_BUFS is a static buffer pool. The index is valid (bounded by
        // NUM_ENDPOINTS * 4). OWN=0 was verified above, so USB hardware won't
        // access this buffer until we set OWN=1 via bdt_write below.
        unsafe {
            EP_BUFS.bufs[idx][..len].copy_from_slice(&buf[..len]);
        }

        // Build descriptor and hand to USB
        let desc = make_bd_desc(
            len as u16,
            state.tx_data_toggle,
            true, // OWN
            true, // DTS
            false,
        );

        // SAFETY: idx is valid, EP_BUFS is static.
        let buf_addr = unsafe { EP_BUFS.bufs[idx].as_ptr() as u32 };
        bdt_write(idx, desc, buf_addr);

        // Toggle for next transfer
        state.tx_data_toggle = !state.tx_data_toggle;
        state.tx_odd = !state.tx_odd;

        Ok(len)
    }

    fn read(&self, ep_addr: EndpointAddress, buf: &mut [u8]) -> usb_device::Result<usize> {
        let ep = ep_addr.index();
        if ep >= NUM_ENDPOINTS {
            return Err(UsbError::InvalidEndpoint);
        }

        let inner = self.inner_mut();
        let state = &mut inner.endpoints[ep];

        if state.ep_type.is_none() {
            return Err(UsbError::InvalidEndpoint);
        }

        if !state.rx_ready {
            return Err(UsbError::WouldBlock);
        }

        let odd = state.rx_complete_odd;
        let idx = bdt_index(ep, false, odd);

        // Read byte count from completed BD
        let bd_desc = bdt_read_desc(idx);
        let count = bd_byte_count(bd_desc) as usize;

        if count > buf.len() {
            return Err(UsbError::BufferOverflow);
        }

        // SAFETY: OWN=0 (rx_ready was set by poll()), so USB hardware won't
        // modify this buffer. The count is validated against buf.len() above.
        unsafe {
            buf[..count].copy_from_slice(&EP_BUFS.bufs[idx][..count]);
        }

        state.rx_ready = false;
        state.rx_setup = false;

        // After SETUP, reset data toggle to DATA1 for the data phase
        if bd_pid(bd_desc) == PID_SETUP {
            state.tx_data_toggle = true;
            state.rx_data_toggle = true;
        } else {
            state.rx_data_toggle = !state.rx_data_toggle;
        }

        // Re-arm the RX buffer on the NEXT odd bank
        let next_odd = state.rx_odd;
        self.arm_rx(ep, next_odd);
        state.rx_odd = !state.rx_odd;

        Ok(count)
    }

    // --- Stall Management ---

    fn set_stalled(&self, ep_addr: EndpointAddress, stalled: bool) {
        let ep = ep_addr.index();
        if ep >= NUM_ENDPOINTS {
            return;
        }

        let inner = self.inner_mut();
        let state = &mut inner.endpoints[ep];

        match ep_addr.direction() {
            UsbDirection::In => state.stalled_in = stalled,
            UsbDirection::Out => state.stalled_out = stalled,
        }

        let usb = usb_regs();

        if stalled {
            usb.endpt(ep).modify(|_, w| w.epstall().set_bit());
        } else {
            usb.endpt(ep).modify(|_, w| w.epstall().clear_bit());

            // Reset data toggle on unstall
            match ep_addr.direction() {
                UsbDirection::In => state.tx_data_toggle = false,
                UsbDirection::Out => {
                    state.rx_data_toggle = false;
                    // Re-arm RX
                    let odd = state.rx_odd;
                    self.arm_rx(ep, odd);
                }
            }
        }
    }

    fn is_stalled(&self, ep_addr: EndpointAddress) -> bool {
        let ep = ep_addr.index();
        if ep >= NUM_ENDPOINTS {
            return false;
        }
        let state = &self.inner().endpoints[ep];
        match ep_addr.direction() {
            UsbDirection::In => state.stalled_in,
            UsbDirection::Out => state.stalled_out,
        }
    }

    // --- Power Management ---

    fn suspend(&self) {
        let usb = usb_regs();
        // Put transceiver into suspend state
        usb.usbctrl().modify(|_, w| w.susp()._1());
    }

    fn resume(&self) {
        let usb = usb_regs();
        // Wake transceiver from suspend
        usb.usbctrl().modify(|_, w| w.susp()._0());
    }

    // --- Polling ---

    fn poll(&self) -> PollResult {
        let usb = usb_regs();
        let istat = usb.istat().read().bits();

        if istat == 0 {
            return PollResult::None;
        }

        // --- USB Reset ---
        if istat & ISTAT_USBRST != 0 {
            // Clear the flag (w1c — write only this bit)
            usb.istat().write(|w| unsafe { w.bits(ISTAT_USBRST) });
            return PollResult::Reset;
        }

        // --- Suspend (sleep) ---
        if istat & ISTAT_SLEEP != 0 {
            usb.istat().write(|w| unsafe { w.bits(ISTAT_SLEEP) });
            return PollResult::Suspend;
        }

        // --- Resume ---
        if istat & ISTAT_RESUME != 0 {
            usb.istat().write(|w| unsafe { w.bits(ISTAT_RESUME) });
            return PollResult::Resume;
        }

        // --- Stall ---
        if istat & ISTAT_STALL != 0 {
            usb.istat().write(|w| unsafe { w.bits(ISTAT_STALL) });
        }

        // --- Error ---
        if istat & ISTAT_ERROR != 0 {
            // Clear all error flags
            let errstat = usb.errstat().read().bits();
            usb.errstat().write(|w| unsafe { w.bits(errstat) });
            usb.istat().write(|w| unsafe { w.bits(ISTAT_ERROR) });
        }

        // --- Token Done ---
        let mut ep_out: u16 = 0;
        let mut ep_in_complete: u16 = 0;
        let mut ep_setup: u16 = 0;

        // Process all pending TOKDNE events
        while usb.istat().read().bits() & ISTAT_TOKDNE != 0 {
            // Read STAT before clearing TOKDNE (STAT is a FIFO)
            let stat = usb.stat().read();
            let ep = stat.endp().bits() as usize;
            let is_tx = stat.tx().is_1();
            let is_odd = stat.odd().bit_is_set();

            // Clear TOKDNE (w1c)
            usb.istat().write(|w| unsafe { w.bits(ISTAT_TOKDNE) });

            if ep >= NUM_ENDPOINTS {
                continue;
            }

            let inner = self.inner_mut();
            let state = &mut inner.endpoints[ep];

            if is_tx {
                // IN transfer complete
                ep_in_complete |= 1 << ep;
            } else {
                // OUT or SETUP received
                let idx = bdt_index(ep, false, is_odd);
                let bd_desc = bdt_read_desc(idx);
                let pid = bd_pid(bd_desc);

                if pid == PID_SETUP {
                    ep_setup |= 1 << ep;
                    state.rx_setup = true;
                    // After SETUP, unfreeze (clear TXSUSPENDTOKENBUSY)
                    usb.ctl().modify(|_, w| w.txsuspendtokenbusy().clear_bit());
                } else {
                    ep_out |= 1 << ep;
                }

                state.rx_ready = true;
                state.rx_complete_odd = is_odd;
            }
        }

        if ep_out != 0 || ep_in_complete != 0 || ep_setup != 0 {
            PollResult::Data {
                ep_out,
                ep_in_complete,
                ep_setup,
            }
        } else {
            PollResult::None
        }
    }

    fn force_reset(&self) -> usb_device::Result<()> {
        let usb = usb_regs();

        // Disable USB to drop D+ pullup
        usb.ctl().modify(|_, w| w.usbensofen()._0());

        // Short delay for host to detect disconnect
        // (~10ms bus timeout, we just need a noticeable gap)
        cortex_m::asm::delay(72_000 * 10);

        // Re-enable USB
        usb.ctl().modify(|_, w| w.usbensofen()._1());

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extension Trait
// ---------------------------------------------------------------------------

/// Extension trait for the USB0 peripheral.
///
/// Consumes the USB0 PAC peripheral, configures the USB 48 MHz clock source,
/// enables the clock gate, and returns a [`UsbBus`] ready for use with
/// `usb_device::bus::UsbBusAllocator`.
pub trait UsbBusExt: Sized {
    /// Consume the USB0 peripheral and return a [`UsbBus`].
    ///
    /// Configures SIM SOPT2 (PLL clock source), CLKDIV2 (USB divider),
    /// and SCGC4 (clock gate). The USB peripheral itself is initialized
    /// later when `usb-device` calls `enable()`.
    fn usb_bus(self, sim: &pac::Sim) -> UsbBus;
}

impl UsbBusExt for pac::Usb0 {
    fn usb_bus(self, sim: &pac::Sim) -> UsbBus {
        // Select PLL as source for USB clock
        sim.sopt2().modify(|_, w| {
            w.pllfllsel().pll()
             .usbsrc()._1()
        });

        // Configure USB clock divider: USB_CLK = PLL × (USBFRAC+1) / (USBDIV+1)
        #[cfg(feature = "mk20d7")]
        {
            // SAFETY: USBDIV is a 3-bit field; value 2 fits.
            // 72 MHz PLL: 72 × 2/3 = 48 MHz (USBFRAC=1, USBDIV=2)
            sim.clkdiv2().write(|w| unsafe {
                w.usbfrac().set_bit()
                 .usbdiv().bits(2)
            });
        }
        #[cfg(feature = "mk20d5")]
        {
            // SAFETY: USBDIV is a 3-bit field; value 0 fits.
            // 48 MHz PLL: 48 × 1/1 = 48 MHz (USBFRAC=0, USBDIV=0)
            sim.clkdiv2().write(|w| unsafe {
                w.usbdiv().bits(0)
            });
        }

        // Enable USB clock gate
        sim.scgc4().modify(|_, w| w.usbotg().enabled());

        UsbBus::new()
    }
}
