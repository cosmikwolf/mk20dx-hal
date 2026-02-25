# Safe DMA API — Design Analysis

## Problem

The current DMA `configure()` methods are `unsafe` because the eDMA engine is an independent bus master that reads/writes memory addresses without Rust's borrow checker. The caller must guarantee:

1. Source/destination addresses are valid and aligned
2. Memory regions remain valid for the entire transfer duration
3. Size and count parameters are internally consistent

This is real unsafety (not svd2rust boilerplate) — if a stack-allocated buffer goes out of scope mid-transfer, the DMA writes to freed memory.

## Goal

A `Transfer` type that encodes buffer ownership in the type system, making it impossible to use a buffer while DMA is accessing it, and returning it when the transfer completes.

## Proposed Design

### embedded-dma Crate

The `embedded-dma` crate (v0.2.0, maintained by rust-embedded team) provides the standard traits:

```rust
// Buffer that DMA reads from (source)
pub unsafe trait ReadBuffer {
    type Word;
    unsafe fn read_buffer(&self) -> (*const Self::Word, usize);
}

// Buffer that DMA writes to (destination)
pub unsafe trait WriteBuffer {
    type Word;
    unsafe fn write_buffer(&mut self) -> (*mut Self::Word, usize);
}
```

These are implemented for `&'static [T]`, `&'static mut [T]`, and types implementing `StableDeref` (heap-allocated containers). In our `no_std` no-alloc context, the primary users would be:

- `&'static [u8]` / `&'static mut [u8]` — linker-placed or `static` buffers
- Fixed-size arrays owned by the `Transfer` struct
- `StaticCell`-allocated buffers

### Transfer Type

```rust
/// An in-progress DMA transfer that owns the channel, buffer, and source config.
///
/// The buffer cannot be accessed while the transfer is active. When complete,
/// call `wait()` to get the channel and buffer back.
pub struct Transfer<const CH: u8, BUF> {
    buffer: BUF,
    _channel: DmaChannel<CH>,  // consumed — prevents reconfiguration
}

impl<const CH: u8, BUF> Transfer<CH, BUF> {
    /// Poll whether the transfer has completed.
    pub fn is_complete(&self) -> bool { ... }

    /// Block until transfer completes, then return the channel and buffer.
    pub fn wait(self) -> (DmaChannel<CH>, BUF) { ... }

    /// Check for DMA errors.
    pub fn has_error(&self) -> bool { ... }
}

/// Abort the transfer on drop to prevent DMA writing to freed memory.
impl<const CH: u8, BUF> Drop for Transfer<CH, BUF> {
    fn drop(&mut self) {
        // Disable ERQ for this channel — stops the DMA immediately
        // This is the critical safety guarantee
        dma_regs().cerq().write(|w| unsafe { w.cerq().bits(CH) });
        compiler_fence(Ordering::SeqCst);
    }
}
```

### Safe Entry Points

```rust
impl<const CH: u8> DmaChannel<CH> {
    /// Memory-to-memory copy. Consumes the channel and source buffer.
    pub fn memcpy<S, D>(self, src: S, dst: D) -> Transfer<CH, (S, D)>
    where
        S: ReadBuffer<Word = u8>,
        D: WriteBuffer<Word = u8>,
    { ... }

    /// Peripheral-to-memory transfer (e.g., FTM CNT → buffer).
    pub fn read_peripheral<P, D>(
        self,
        periph: P,
        dst: D,
        source: DmaSource,
    ) -> Transfer<CH, (P, D)>
    where
        P: DmaReadable,     // new trait for peripheral registers
        D: WriteBuffer,
    { ... }

    /// Memory-to-peripheral transfer (e.g., buffer → FTM CnV).
    pub fn write_peripheral<S, P>(
        self,
        src: S,
        periph: P,
        source: DmaSource,
    ) -> Transfer<CH, (S, P)>
    where
        S: ReadBuffer,
        P: DmaWritable,     // new trait for peripheral registers
    { ... }
}
```

### Peripheral Register Wrapper

Half the FTM+DMA patterns use peripheral register addresses (FTM0_C0V, FTM0_CNT). embedded-dma traits model memory buffers, not MMIO. We need a wrapper:

```rust
/// A peripheral register that DMA can read from.
///
/// Wraps a fixed MMIO address. Implements the DMA source interface
/// so it can be used in `read_peripheral()` transfers.
pub struct PeripheralRead<T: Copy> {
    addr: u32,
    _phantom: PhantomData<T>,
}

/// A peripheral register that DMA can write to.
pub struct PeripheralWrite<T: Copy> {
    addr: u32,
    _phantom: PhantomData<T>,
}

// FTM channels could provide these:
impl<FTM: FtmInstance, const CH: u8> FtmChannel<FTM, CH> {
    /// Get a DMA-readable handle to this channel's CnV register.
    pub fn dma_read_value(&self) -> PeripheralRead<u16> { ... }

    /// Get a DMA-writable handle to this channel's CnV register.
    pub fn dma_write_value(&self) -> PeripheralWrite<u16> { ... }
}

impl<FTM: FtmInstance> FtmTimer<FTM> {
    /// Get a DMA-readable handle to the CNT register.
    pub fn dma_read_counter(&self) -> PeripheralRead<u16> { ... }
}
```

### What the FTM+DMA Test Would Look Like

```rust
// Current (unsafe):
unsafe {
    state.dma.ch0.configure(&TransferConfig {
        source_addr: &sentinel as *const u32 as u32,
        dest_addr: &mut dest as *mut u32 as u32,
        ...
    });
}
state.dma.ch0.set_source(DmaSource::FTM0_CH0);
state.dma.ch0.enable_request();
// ... poll ...
// BUG RISK: sentinel/dest could go out of scope

// Safe API:
let xfer = state.dma.ch0.read_peripheral(
    ftm0.ch0.dma_read_value(),
    &mut captures_buf,           // WriteBuffer
    DmaSource::FTM0_CH0,
);
let (ch0, (_periph, captures)) = xfer.wait();
// captures_buf was inaccessible during transfer — no aliasing possible
```

## Challenges

### 1. no_std / no-alloc Buffer Ownership

Without a heap, buffers are typically stack-local arrays or `static mut`. Consuming a stack array into `Transfer` is fine if the `Transfer` lives in the same scope. But `static mut` requires `unsafe` to access regardless.

Practical workaround: most embedded DMA buffers are declared `static` and accessed via `StaticCell` or `singleton!()` macros. This is already the pattern in embassy and RTIC.

### 2. Reusable Channel Patterns

The current tests configure a DMA channel once and reuse it across multiple tests. The `Transfer` pattern returns the channel on completion, so reuse is possible but requires rebinding:

```rust
let (ch0, _buf) = xfer.wait();
// ch0 is available again for a new transfer
let xfer2 = ch0.read_peripheral(...);
```

### 3. Circular / Continuous Transfers

Some DMA patterns run continuously (e.g., ADC scan → ring buffer). The `Transfer` type needs a `CircularTransfer` variant that allows reading completed portions while DMA continues writing. This is more complex — embassy-stm32 handles this with double-buffering and a `readable_half()` method.

### 4. Scatter-Gather

The existing `configure_scatter_gather()` uses linked TCD chains. This is the hardest pattern to make safe because the TCD chain references memory that must remain valid for an indefinite series of transfers. A `ScatterGatherTransfer` type would need to own the entire TCD chain plus all referenced buffers.

### 5. Migration Path

The `unsafe` low-level API should remain available (renamed to `configure_raw()` or similar) for patterns that can't be expressed through the safe API. The safe API is a layer on top, not a replacement.

## Implementation Order

1. Add `embedded-dma` dependency
2. Implement `Transfer` type with `Drop`-based abort safety
3. Add `memcpy()` safe entry point (simplest case)
4. Add `PeripheralRead`/`PeripheralWrite` types
5. Add `read_peripheral()` / `write_peripheral()` entry points
6. Add `FtmChannel::dma_read_value()` / `dma_write_value()` helpers
7. Update FTM+DMA tests to use safe API where possible
8. Keep `configure()` as `configure_raw()` for advanced patterns
9. Future: `CircularTransfer` for continuous modes

## References

- [embedded-dma crate](https://docs.rs/embedded-dma/0.2.0/embedded_dma/) — ReadBuffer/WriteBuffer traits
- [The Embedonomicon: DMA](https://docs.rust-embedded.org/embedonomicon/dma.html) — canonical DMA safety analysis
- [stm32f4xx-hal DMA](https://github.com/stm32-rs/stm32f4xx-hal/blob/master/src/dma/mod.rs) — Transfer pattern reference
- [embassy-stm32 DMA](https://github.com/embassy-rs/embassy/tree/main/embassy-stm32/src/dma) — async Transfer pattern
- [imxrt-hal DMA](https://github.com/imxrt-rs/imxrt-hal/tree/master/src/common/dma) — NXP cousin reference
