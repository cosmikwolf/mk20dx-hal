//! USB device driver for the Kinetis USB-FS controller.
//!
//! Implements the [`usb_device::bus::UsbBus`] trait using the hardware BDT
//! (Buffer Descriptor Table) with ping-pong buffering. Supports 16 endpoints,
//! each bidirectional with EVEN/ODD banks.
//!
//! Both MK20D5 (Teensy 3.0) and MK20D7 (Teensy 3.1/3.2) have identical USB0
//! peripherals — no `#[cfg]` gating needed.
//!
//! # Usage: polling
//!
//! ```ignore
//! use mk20dx_hal::prelude::*;
//! use usb_device::prelude::*;
//!
//! let dp = pac::Peripherals::take().unwrap();
//! let clocks = dp.mcg.constrain().freeze(dp.osc, &dp.sim);
//!
//! let usb_bus = dp.usb0.usb_bus(&dp.sim, &clocks);
//! let usb_bus_alloc = UsbBusAllocator::new(usb_bus);
//!
//! let mut usb_dev = UsbDeviceBuilder::new(&usb_bus_alloc, UsbVidPid(0x16c0, 0x27dd))
//!     .build();
//!
//! loop {
//!     usb_dev.poll(&mut []);
//! }
//! ```
//!
//! # Usage: interrupt-driven (RTIC)
//!
//! `poll()` can run in the USB0 interrupt handler instead. The USB0 line is
//! asserted while any `ISTAT` flag that `INTEN` enables is set, so a flag
//! that `poll()` leaves set makes the handler run again at once. `poll()`
//! uses that on purpose: it reports one SETUP per call (see the note in
//! [`UsbBus::poll`](usb_device::bus::UsbBus::poll)) and leaves later tokens
//! for the next run.
//!
//! [`UsbBus::enable_sof_interrupt`] adds the 1 ms start-of-frame token as an
//! interrupt source. [`UsbBus::take_sof`] then tells the handler that a frame
//! started, which is a good tick to send queued IN packets.
//!
//! ```ignore
//! #[rtic::app(device = mk20dx_hal::pac, peripherals = true)]
//! mod app {
//!     use mk20dx_hal::prelude::*;
//!     use mk20dx_hal::usb::UsbBus;
//!     use usb_device::class_prelude::UsbBusAllocator;
//!     use usb_device::prelude::*;
//!     use usbd_midi::UsbMidiClass;
//!
//!     #[shared]
//!     struct Shared {}
//!
//!     #[local]
//!     struct Local {
//!         usb_dev: UsbDevice<'static, UsbBus>,
//!         midi: UsbMidiClass<'static, UsbBus>,
//!     }
//!
//!     #[init(local = [usb_alloc: Option<UsbBusAllocator<UsbBus>> = None])]
//!     fn init(cx: init::Context) -> (Shared, Local) {
//!         let dp = cx.device;
//!         // The WDOG unlock belongs in #[pre_init], not here: see `watchdog`.
//!         let clocks = dp.mcg.constrain().freeze(dp.osc, &dp.sim);
//!
//!         let usb_bus = dp.usb0.usb_bus(&dp.sim, &clocks);
//!         usb_bus.enable_sof_interrupt(true);
//!         let alloc: &'static _ = cx.local.usb_alloc.insert(UsbBusAllocator::new(usb_bus));
//!
//!         let midi = UsbMidiClass::new(alloc, 1, 1).unwrap();
//!         let usb_dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x16c0, 0x0485)).build();
//!
//!         // RTIC unmasks USB0 when init returns. Nothing calls the bus before that.
//!         (Shared {}, Local { usb_dev, midi })
//!     }
//!
//!     // The only context that touches the bus: see the safety note on `UsbBus`.
//!     #[task(binds = USB0, priority = 2, local = [usb_dev, midi])]
//!     fn usb0(cx: usb0::Context) {
//!         let usb_dev = cx.local.usb_dev;
//!         let midi = cx.local.midi;
//!         usb_dev.poll(&mut [&mut *midi]);
//!
//!         let mut buf = [0u8; 64];
//!         if let Ok(n) = midi.read(&mut buf) {
//!             // Read OUT data here. TOKDNE is already clear, so no second
//!             // interrupt comes for data that this run does not read.
//!             let _ = &buf[..n];
//!         }
//!
//!         if usb_dev.bus().take_sof() {
//!             // A frame started (1 ms). Send the queued IN packets here.
//!         }
//!     }
//! }
//! ```
//!
//! Without RTIC, put the same body in a `#[cortex_m_rt::interrupt] fn USB0()`
//! and unmask `pac::Interrupt::USB0` in the NVIC after the device is built.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::clocks::Clocks;
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
const ISTAT_SOFTOK: u8 = 1 << 2;
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

/// Read-modify-write `INTEN` with interrupts masked. `poll()` changes
/// RESUMEEN from the USB context while `enable_sof_interrupt()` may change
/// SOFTOKEN from another one; the critical section keeps both changes.
fn inten_modify(
    f: impl FnOnce(&mut pac::usb0::inten::W) -> &mut pac::usb0::inten::W,
) {
    cortex_m::interrupt::free(|_| {
        usb_regs().inten().modify(|_, w| f(w));
    });
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
    // DATA0/DATA1 toggle tracking, TX only. The RX toggle follows from the
    // bank: see rx_bank_data1().
    tx_data_toggle: bool,
    // Ping-pong bank tracking. TX only: firmware picks the TX bank, but the
    // controller picks the RX bank, so RX needs no counter — see read().
    tx_odd: bool,
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
            tx_odd: false,
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
///
/// # One context only
///
/// Every `usb_device` call on this bus (`UsbDevice::poll`, class reads and
/// writes, endpoint calls) must come from one execution context: either a
/// polling loop, or the USB0 interrupt handler. Do not mix them, and do not
/// call the bus from a second interrupt. The driver does not lock its
/// endpoint state.
///
/// [`enable_sof_interrupt`](Self::enable_sof_interrupt),
/// [`take_sof`](Self::take_sof) and [`frame_number`](Self::frame_number) are
/// the exceptions: they touch only atomics and registers, so any context can
/// call them.
pub struct UsbBus {
    inner: UnsafeCell<Inner>,
    /// SOFTOKEN is wanted in `INTEN`. Kept here because `enable()` resets the
    /// module, which clears `INTEN`, and must then write it back.
    sof_irq: AtomicBool,
    /// A SOF token arrived since the last `take_sof()`. Set by `poll()`.
    sof_pending: AtomicBool,
}

// SAFETY: Single-core Cortex-M4. The application calls the bus from a single
// context only — the polling loop or the USB0 interrupt handler, never both
// (see "One context only" above). The UnsafeCell interior mutability is
// safe because there is then no concurrent access. The fields that other
// contexts may touch are atomics.
unsafe impl Send for UsbBus {}
unsafe impl Sync for UsbBus {}

impl UsbBus {
    fn new() -> Self {
        Self {
            inner: UnsafeCell::new(Inner::new()),
            sof_irq: AtomicBool::new(false),
            sof_pending: AtomicBool::new(false),
        }
    }

    /// Enable or disable the start-of-frame (SOF) interrupt.
    ///
    /// The host sends a SOF token every 1 ms while the bus is active. With
    /// this on, each one runs the USB0 handler, and [`take_sof`](Self::take_sof)
    /// returns `true` after the next `poll()`. Off by default.
    ///
    /// Call it before or after the device is built. The setting survives a
    /// USB bus reset.
    pub fn enable_sof_interrupt(&self, enable: bool) {
        self.sof_irq.store(enable, Ordering::Relaxed);
        // The clock gate is on: `usb_bus()` set it before this value exists.
        inten_modify(|w| w.softoken().bit(enable));
    }

    /// Return `true` once for each `poll()` that saw a SOF token, and clear the flag.
    ///
    /// Several frames between two calls give one `true`. Use
    /// [`frame_number`](Self::frame_number) to count them.
    pub fn take_sof(&self) -> bool {
        self.sof_pending.swap(false, Ordering::Relaxed)
    }

    /// The 11-bit frame number of the last SOF token (`FRMNUMH:FRMNUML`).
    ///
    /// It increments once each 1 ms and wraps at 2048.
    pub fn frame_number(&self) -> u16 {
        let usb = usb_regs();
        // FRMNUML first, then FRMNUMH; reread the low byte to catch a carry
        // between the two reads.
        loop {
            let lo = usb.frmnuml().read().bits();
            let hi = usb.frmnumh().read().bits();
            if usb.frmnuml().read().bits() == lo {
                return (u16::from(hi & 0x07) << 8) | u16::from(lo);
            }
        }
    }

    fn inner(&self) -> &Inner {
        // SAFETY: Single-core Cortex-M4. The application calls UsbBusTrait
        // methods from one context only, the polling loop or the USB0
        // interrupt handler. No concurrent access occurs.
        unsafe { &*self.inner.get() }
    }

    // `UsbBus` must hand out `&mut Inner` from `&self`: every `UsbBusTrait`
    // method takes `&self`, yet the driver has to mutate endpoint state. The
    // aliasing rule is upheld by the caller, not the type system — see the
    // `inner()` safety note — so the lint is allowed here rather than papered
    // over with a `RefCell` whose borrow checks could panic inside an ISR.
    #[allow(clippy::mut_from_ref)]
    fn inner_mut(&self) -> &mut Inner {
        // SAFETY: Same invariant as inner(). Single-core; one context (the
        // polling loop or the USB0 handler) calls the bus.
        unsafe { &mut *self.inner.get() }
    }

    /// Stop the RESUME interrupt and drop a stale RESUME flag. Outside
    /// suspend the flag follows DP/DM activity, so it must stay off.
    fn disable_resume_irq(&self) {
        inten_modify(|w| w.resumeen()._0());
        usb_regs().istat().write(|w| unsafe { w.bits(ISTAT_RESUME) });
    }

    /// Arm an RX buffer descriptor for the given endpoint, expecting `data1`.
    fn arm_rx(&self, ep: usize, odd: bool, data1: bool) {
        let state = &self.inner().endpoints[ep];
        let idx = bdt_index(ep, false, odd);
        let desc = make_bd_desc(
            state.max_packet_size,
            data1,
            true, // OWN = USB
            true, // DTS
            false,
        );
        // SAFETY: EP_BUFS is a static buffer pool that exists for the program's
        // lifetime. The index is computed from a valid endpoint number.
        let buf_addr = unsafe { EP_BUFS.bufs[idx].as_ptr() as u32 };
        bdt_write(idx, desc, buf_addr);
    }

    /// Arm both RX banks of an endpoint, as after a bus reset.
    ///
    /// With DTS set, a packet whose DATA0/DATA1 PID does not match the bank
    /// is dropped. The controller alternates banks and the host alternates
    /// the PID, so on a bulk or interrupt endpoint EVEN always takes DATA0
    /// and ODD always takes DATA1 (usb_dev.c: `BDT_DESC(64, 0)` on EVEN,
    /// `BDT_DESC(64, 1)` on ODD). Arming both with DATA0 drops every
    /// second OUT packet. A control endpoint starts with DATA0 in both
    /// banks for the SETUP (usb_dev.c USBRST handler).
    fn arm_rx_both(&self, ep: usize) {
        let control = self.inner().endpoints[ep].ep_type == Some(EndpointType::Control);
        self.arm_rx(ep, false, false);
        self.arm_rx(ep, true, !control);
    }

    /// The PID that the next packet into this RX bank carries, after the
    /// bank completed a packet.
    ///
    /// Bulk and interrupt: fixed by the bank, as in `arm_rx_both()`. Control:
    /// DATA1 for every re-arm, which is what usb_dev.c does after a SETUP
    /// and after an OUT on EP0 (`BDT_DESC(EP0_SIZE, DATA1)`). A SETUP is
    /// received whatever the bank expects.
    fn rx_bank_data1(&self, ep: usize, odd: bool) -> bool {
        self.inner().endpoints[ep].ep_type == Some(EndpointType::Control) || odd
    }

    /// Configure endpoint 0 for control transfers after reset.
    fn configure_ep0(&self) {
        let inner = self.inner_mut();
        let ep0 = &mut inner.endpoints[0];
        ep0.tx_data_toggle = false;
        ep0.tx_odd = false;
        ep0.rx_ready = false;
        ep0.rx_setup = false;
        ep0.stalled_in = false;
        ep0.stalled_out = false;

        // Arm BOTH RX banks. The controller ping-pongs between them on its
        // own; firmware never chooses the bank. The reference driver
        // (Teensy usb_dev.c, USBRST handler) arms EVEN and ODD here for the
        // same reason. Arming only EVEN means the second packet the host
        // sends lands on an unowned ODD bank and is never received.
        self.arm_rx(0, false, false);
        self.arm_rx(0, true, false);

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
    // The Kinetis USB-FS latches ADDR as soon as it is written, so the write
    // must happen *after* the status-stage IN is acknowledged — a write during
    // the data stage would move the device mid-transfer and the host would
    // never see the handshake. `false` asks usb-device for exactly that
    // ordering. The reference driver does the same: usb_dev.c writes
    // `USB0_ADDR = setup.wValue` in its "IN transaction completed" case, not
    // in usb_setup(), where SET_ADDRESS is an empty arm of the switch.
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
        // The divider was programmed by `usb_bus()`, which has the `Clocks`
        // token needed to pick it. Nothing to redo here.

        // Enable USB clock gate
        sim.scgc4().modify(|_, w| w.usbotg().enabled());

        // --- Initialize USB peripheral ---

        // SAFETY: bdtba fields accept the respective address bits. The BDT
        // static is 512-byte aligned (repr(C, align(512))).
        // Reset the USB module before configuring it, as the reference
        // driver does (USBTRC0[USBRESET], self-clearing). Without it the SIE
        // can come up stalled (CTL[TXSUSPENDTOKENBUSY] stuck set).
        // SAFETY: USBTRC0 is an 8-bit register; 0x80 is USBRESET.
        usb.usbtrc0().write(|w| unsafe { w.bits(0x80) });
        while usb.usbtrc0().read().bits() & 0x80 != 0 {}

        // Undocumented bit the reference driver sets before enabling the
        // module (Freescale/Teensy usb_init: USB0_USBTRC0 |= 0x40).
        // SAFETY: 8-bit register, valid value.
        usb.usbtrc0().write(|w| unsafe { w.bits(0x40) });

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

        // Enable interrupts: reset, token done, sleep, stall, error, and SOF
        // if the application asked for it. RESUME stays off: K20 RM ISTAT
        // says "When not in suspend mode this interrupt must be disabled",
        // because the flag follows DP/DM activity. suspend() turns it on.
        // Matches usb_dev.c, whose INTEN has no RESUMEEN.
        let sof = self.sof_irq.load(Ordering::Relaxed);
        usb.inten().write(|w| {
            w.usbrsten()._1()
             .erroren()._1()
             .tokdneen()._1()
             .sleepen()._1()
             .stallen()._1()
             .softoken().bit(sof)
        });

        // Disable weak pulldowns, not in suspend
        usb.usbctrl().write(|w| w.pde()._0().susp()._0());

        // Configure EP0
        self.configure_ep0();

        // Enable USB module (non-OTG mode: D+ pullup is automatic)
        usb.ctl().write(|w| w.usbensofen()._1());

        // Use non-OTG mode: OTGEN=0 means the D+ pullup is controlled by CONTROL
        usb.otgctl().write(|w| w.otgen()._0());

        // Enable the D+ pullup: this is what signals the attach to the host.
        // Without it the device never enumerates.
        usb.control().write(|w| w.dppullupnonotg().set_bit());
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

        // CTL is written whole, never read-modify-written.
        // CTL[TXSUSPENDTOKENBUSY] reads as 1 while the controller is frozen on
        // a SETUP token; a read-modify-write feeds that bit straight back and
        // re-asserts the freeze, so the device stops answering the host and
        // never recovers. usb_dev.c only ever assigns CTL whole, for this
        // reason. `write` yields the reset value (0) for every field not named.
        // Reset the ping-pong ODD toggle, then re-enable. Matches
        // usb_dev.c's `USB0_CTL = USB_CTL_ODDRST` / `= USB_CTL_USBENSOFEN`.
        usb.ctl().write(|w| w.oddrst().set_bit());
        usb.ctl().write(|w| w.usbensofen()._1());

        // Reset all endpoint state
        for i in 0..NUM_ENDPOINTS {
            inner.endpoints[i].tx_data_toggle = false;
            inner.endpoints[i].tx_odd = false;
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

            // Arm both RX banks — see configure_ep0() and arm_rx_both().
            self.arm_rx_both(i);

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

        // A bus reset also ends suspend: RESUME goes off again.
        inten_modify(|w| w.resumeen()._0());

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

        if bd_pid(bd_desc) == PID_SETUP {
            // A SETUP cancels whatever control transfer was in flight. Drop
            // any TX descriptor still owned by the controller, or a leftover
            // IN from an abandoned transfer answers the next IN token with
            // stale data. Matches usb_dev.c, which zeroes both EP0 TX BDs
            // on every SETUP.
            bdt_write(bdt_index(ep, true, false), 0, 0);
            bdt_write(bdt_index(ep, true, true), 0, 0);
            // NOTE: `tx_odd` is deliberately NOT reset here. Clearing the
            // descriptors does not move the controller's own TX ping-pong
            // pointer — it never consumed those banks — so forcing the
            // software pointer back to EVEN would desynchronise the two.
            // usb_dev.c leaves `ep0_tx_bdt_bank` alone on SETUP for the same
            // reason; it only resets it on bus reset, alongside ODDRST.
            // The first IN after a SETUP is always DATA1.
            state.tx_data_toggle = true;
        }

        // Re-arm the bank that just completed. The controller owns the
        // ping-pong sequence, so the bank to give back is always the one it
        // just handed us (`rx_complete_odd`) — never a separately tracked
        // counter, which desynchronises after the first packet.
        self.arm_rx(ep, odd, self.rx_bank_data1(ep, odd));

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
                    // Re-arm both banks, as after a reset. The PID stays
                    // tied to the bank (see arm_rx_both()), as in usb_dev.c,
                    // which does not touch the RX BDs on CLEAR_FEATURE.
                    self.arm_rx_both(ep);
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
        // Deliberately does NOT set USBCTRL[SUSP]. On this controller that
        // bit makes the transceiver ignore everything but resume signalling,
        // so a host bus reset after the idle gap that follows attach is never
        // seen and the device never enumerates (observed: USBCTRL 0x80 with
        // ADDR 0 forever). The reference driver writes USBCTRL once at init
        // and never again.

        // Now in suspend, so RESUME is the wake-up signal: turn it on. Clear
        // the flag first, or activity from before the suspend reports a
        // resume at once.
        usb_regs().istat().write(|w| unsafe { w.bits(ISTAT_RESUME) });
        inten_modify(|w| w.resumeen()._1());
    }

    fn resume(&self) {
        let usb = usb_regs();
        // Wake transceiver from suspend
        usb.usbctrl().modify(|_, w| w.susp()._0());
        // Out of suspend: RESUME must be off again (see enable()). usb-device
        // calls this for any event that ends a suspend, not only RESUME.
        self.disable_resume_irq();
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

        // NOTE: SLEEP and RESUME are deliberately handled *after* the token
        // loop below, not here. A SETUP token freezes the controller
        // (CTL[TXSUSPENDTOKENBUSY]) until firmware clears the freeze, which
        // only happens in that loop. Returning Suspend first livelocks the
        // device: frozen -> no response -> bus goes idle -> SLEEP sets ->
        // poll returns early again -> the freeze is never cleared. The
        // reference driver services TOKDNE before USBRST and SLEEP for the
        // same reason (usb_dev.c: TOKDNE at the top of the ISR, USBRST far
        // below it).

        // Interrupt mode: every flag that INTEN enables is cleared on some
        // path of this function, or the USB0 handler runs forever. USBRST
        // returns above with the rest still set; the handler runs again for
        // them. The SETUP `break` below leaves TOKDNE set on purpose, for the
        // same reason. SLEEP and RESUME wait while a token is reported.

        // --- Start of frame ---
        // It must be cleared or ISTAT never reads zero. Record it for
        // take_sof(); SOFTOK sets in ISTAT whether SOFTOKEN is on or not.
        if istat & ISTAT_SOFTOK != 0 {
            usb.istat().write(|w| unsafe { w.bits(ISTAT_SOFTOK) });
            self.sof_pending.store(true, Ordering::Relaxed);
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
                    state.rx_ready = true;
                    state.rx_complete_odd = is_odd;
                    // After SETUP, unfreeze (clear TXSUSPENDTOKENBUSY).
                    // Whole-register write — see reset(). This both clears
                    // the freeze and leaves USBENSOFEN set.
                    usb.ctl().write(|w| w.usbensofen()._1());
                    // Stop here. Clearing the freeze lets the controller run
                    // again, so continuing the loop could consume the status
                    // stage of this very transfer before usb-device has read
                    // the SETUP — and a second SETUP would overwrite
                    // `rx_complete_odd`, losing the first packet. The
                    // reference driver handles one token per interrupt for
                    // the same reason. Remaining tokens are picked up by the
                    // next poll().
                    break;
                }

                ep_out |= 1 << ep;
                state.rx_ready = true;
                state.rx_complete_odd = is_odd;
            }
        }

        if ep_out != 0 || ep_in_complete != 0 || ep_setup != 0 {
            return PollResult::Data {
                ep_out,
                ep_in_complete,
                ep_setup,
            };
        }

        // Only once no token is outstanding is it safe to report a bus state
        // change — see the note above.
        let istat = usb.istat().read().bits();

        if istat & ISTAT_SLEEP != 0 {
            usb.istat().write(|w| unsafe { w.bits(ISTAT_SLEEP) });
            return PollResult::Suspend;
        }

        if istat & ISTAT_RESUME != 0 {
            usb.istat().write(|w| unsafe { w.bits(ISTAT_RESUME) });
            // The flag sets on any DP/DM activity, also outside suspend.
            // Report it only while suspend() has RESUMEEN on.
            if usb.inten().read().resumeen().bit_is_set() {
                return PollResult::Resume;
            }
        }

        PollResult::None
    }

    fn force_reset(&self) -> usb_device::Result<()> {
        let usb = usb_regs();

        // Disable USB to drop D+ pullup (whole-register write — see reset()).
        usb.ctl().write(|w| w.usbensofen()._0());

        // Short delay for host to detect disconnect
        // (~10ms bus timeout, we just need a noticeable gap)
        cortex_m::asm::delay(72_000 * 10);

        // Re-enable USB (whole-register write — see reset()).
        usb.ctl().write(|w| w.usbensofen()._1());

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extension Trait
// ---------------------------------------------------------------------------

/// Extension trait for the USB0 peripheral.
///
/// USB clock divider settings: `USB_CLK = PLL x (USBFRAC+1) / (USBDIV+1)`,
/// which must land on exactly 48 MHz.
///
/// Derived from the configured PLL rate rather than hard-coded, because the
/// mk20d7 part runs at 72, 96 or 120 MHz depending on which `freeze_at`
/// preset was used, and each needs a different divider. Getting this wrong
/// does not fail loudly — the module simply clocks at the wrong rate and the
/// host never enumerates the device.
#[cfg(feature = "mk20d7")]
const fn usb_clk_divider(pll_hz: u32) -> Option<(bool, u8)> {
    match pll_hz {
        // 72 x 2/3 = 48
        72_000_000 => Some((true, 2)),
        // 96 x 1/2 = 48
        96_000_000 => Some((false, 1)),
        // 120 x 2/5 = 48
        120_000_000 => Some((true, 4)),
        _ => None,
    }
}

/// Consumes the USB0 PAC peripheral, configures the USB 48 MHz clock source,
/// enables the clock gate, and returns a [`UsbBus`] ready for use with
/// `usb_device::bus::UsbBusAllocator`.
pub trait UsbBusExt: Sized {
    /// Consume the USB0 peripheral and return a [`UsbBus`].
    ///
    /// Configures SIM SOPT2 (PLL clock source), CLKDIV2 (USB divider),
    /// and SCGC4 (clock gate). The USB peripheral itself is initialized
    /// later when `usb-device` calls `enable()`.
    fn usb_bus(self, sim: &pac::Sim, clocks: &Clocks) -> UsbBus;
}

impl UsbBusExt for pac::Usb0 {
    fn usb_bus(self, sim: &pac::Sim, clocks: &Clocks) -> UsbBus {
        // Select PLL as source for USB clock
        sim.sopt2().modify(|_, w| {
            w.pllfllsel().pll()
             .usbsrc()._1()
        });

        // Configure USB clock divider: USB_CLK = PLL × (USBFRAC+1) / (USBDIV+1)
        #[cfg(feature = "mk20d7")]
        {
            let (frac, div) = match usb_clk_divider(clocks.core_clk().to_raw()) {
                Some(v) => v,
                None => panic!("USB needs a 72, 96 or 120 MHz PLL to reach 48 MHz"),
            };
            // SAFETY: USBDIV is a 3-bit field; usb_clk_divider only yields 1, 2 or 4.
            sim.clkdiv2().write(|w| unsafe {
                w.usbfrac().bit(frac)
                 .usbdiv().bits(div)
            });
        }
        #[cfg(feature = "mk20d5")]
        {
            let _ = clocks;
            // SAFETY: USBDIV is a 3-bit field; value 0 fits.
            // 48 MHz PLL: 48 x 1/1 = 48 MHz (USBFRAC=0, USBDIV=0)
            sim.clkdiv2().write(|w| unsafe {
                w.usbdiv().bits(0)
            });
        }

        // Enable USB clock gate
        sim.scgc4().modify(|_, w| w.usbotg().enabled());

        UsbBus::new()
    }
}
