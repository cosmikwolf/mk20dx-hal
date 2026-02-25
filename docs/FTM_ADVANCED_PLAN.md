# FTM Advanced Features — Implementation Plan

## Foundation: What We Have Now

The split pattern (`FtmExt::split()`) gives us `FtmTimer<FTM>` + `FtmChannel<FTM, CH>` handles. All current functionality runs in **legacy mode (FTMEN=0)**: edge-aligned PWM, output compare, input capture. The standalone `OutputCompare` and `InputCapture` types still compile but are superseded by the unified `FtmChannel`.

## Feature Dependency Graph

```
FTMEN=0 (current)              FTMEN=1 (enhanced)
├── Edge-aligned PWM (EPWM)    ├── Combine mode
├── Center-aligned PWM (CPWMS) │   ├── Complementary outputs (COMPm)
├── Output Compare             │   ├── Dead time insertion (DTENm)
├── Input Capture              │   ├── Asymmetric PWM
│                              │   └── PWM sync of CnV (SYNCENm)
│                              ├── Fault protection (FAULTM)
│                              ├── Output polarity (POL)
│                              ├── Enhanced sync (SYNCONF)
│                              │   ├── Coherent multi-CnV updates
│                              │   └── DMA-driven waveform generation
│                              └── Dual Edge Capture (DECAPEN)
```

**Critical rule from refman:** Do not write to FTM-specific registers (CNTIN through PWMLOAD, offsets 0x4C–0x98) when FTMEN=0.

## Phase 1: Center-Aligned PWM (FTMEN=0)

**Complexity:** Low — single SC register bit, no new types needed.

Center-aligned PWM uses up-down counting. The counter counts CNTIN→MOD→CNTIN, producing symmetric pulses centered in the period. Period = 2×(MOD−CNTIN) ticks, pulse width = 2×(CnV−CNTIN) ticks.

### API Design

```rust
/// PWM alignment mode.
pub enum PwmAlignment {
    /// Edge-aligned: counter counts up, wraps at MOD. (Default)
    EdgeAligned,
    /// Center-aligned: counter counts up-down. Symmetric pulses.
    /// MOD must be in 0x0001..=0x7FFF.
    CenterAligned,
}

impl<FTM: FtmInstance> FtmTimer<FTM> {
    /// Set the PWM alignment mode.
    ///
    /// Stops the counter, changes CPWMS, restarts. All channels on this
    /// FTM must use the same alignment — you cannot mix edge-aligned and
    /// center-aligned channels.
    pub fn set_alignment(&mut self, align: PwmAlignment);
}
```

### Constraints to Enforce
- MOD must be ≤ 0x7FFF in center-aligned mode (values above give ambiguous results)
- Incompatible with combine mode (`CPWMS=1` requires `COMBINE=0`)
- `set_frequency()` must account for the 2× period factor

### Register Changes
- `SC.CPWMS`: 0=edge-aligned (up only), 1=center-aligned (up-down)
- Write-protected — requires `MODE.WPDIS=1` (already done in `split()`)

---

## Phase 2: Output Polarity (FTMEN=1 required)

**Complexity:** Low — but this is the first feature requiring FTMEN=1, so it forces the FTMEN transition design.

One bit per channel in the POL register. `POLn=0` = active-high (default), `POLn=1` = active-low. Affects fault safe values and deadtime behavior.

### The FTMEN=0 → FTMEN=1 Transition

This is the key architectural decision. Options:

**Option A: Mode parameter on `split()`**
```rust
pub enum FtmMode {
    /// Legacy TPM-compatible mode. Supports EPWM, CPWM, OC, IC.
    Legacy,
    /// Enhanced mode. Adds combine, deadtime, fault, polarity, sync.
    Enhanced,
}

let ftm0 = dp.ftm0.split(FtmMode::Enhanced, &clocks, &dp.sim);
```
Pro: Explicit. Con: Breaks existing `split()` signature.

**Option B: Separate `split_enhanced()` entry point**
```rust
let ftm0 = dp.ftm0.split(&clocks, &dp.sim);          // FTMEN=0
let ftm0 = dp.ftm0.split_enhanced(&clocks, &dp.sim);  // FTMEN=1
```
Pro: Backward compatible. Con: Two entry points.

**Option C: Auto-upgrade on first enhanced feature call**
```rust
let ftm0 = dp.ftm0.split(&clocks, &dp.sim);
ftm0.timer.set_polarity(7, true);  // internally sets FTMEN=1 if not already
```
Pro: No API change. Con: Hidden state transition, surprising behavior.

**Recommendation: Option B.** `split()` stays legacy. `split_enhanced()` sets FTMEN=1 and returns an `Ftm0PartsEnhanced` (or same type with a typestate marker). Enhanced-only methods are only available on the enhanced variant.

Actually, simpler — use a typestate on `FtmTimer`:

```rust
pub struct Legacy;
pub struct Enhanced;

pub struct FtmTimer<FTM, MODE = Legacy> { ... }

// Legacy methods available on both
impl<FTM, MODE> FtmTimer<FTM, MODE> {
    pub fn stop(&mut self);
    pub fn start(&mut self);
    pub fn set_modulo(&mut self, mod_val: u16);
    // ...
}

// Enhanced-only methods
impl<FTM> FtmTimer<FTM, Enhanced> {
    pub fn set_polarity(&mut self, channel: u8, inverted: bool);
    pub fn configure_combine(&mut self, pair: u8, config: CombineConfig);
    pub fn set_deadtime(&mut self, config: DeadtimeConfig);
    pub fn sync_trigger(&mut self);
    // ...
}
```

`split()` returns `FtmTimer<FTM, Legacy>`, `split_enhanced()` returns `FtmTimer<FTM, Enhanced>`. The `FtmChannel` type doesn't need the mode marker since channel operations (set_pwm, set_value, etc.) work in both modes.

### API Design

```rust
impl<FTM: FtmInstance> FtmTimer<FTM, Enhanced> {
    /// Set output polarity for a channel.
    ///
    /// When `inverted` is true, the channel output is active-low (POLn=1).
    /// Default is active-high (POLn=0).
    pub fn set_polarity(&mut self, channel: u8, inverted: bool);
}
```

### Register Changes
- `MODE.FTMEN`: must be 1
- `POL` register: one bit per channel, write-protected

---

## Phase 3: Combine Mode + Complementary Outputs

**Complexity:** Medium — new channel-pair abstraction, several register interactions.

Combine mode pairs even+odd channels (0+1, 2+3, 4+5, 6+7). CnV controls rising edge, C(n+1)V controls falling edge. With COMPm=1, the odd channel output is the complement of the even channel.

### API Design

```rust
/// Configuration for a combined channel pair.
pub struct CombineConfig {
    /// Enable complementary output on odd channel.
    pub complementary: bool,
    /// Enable dead time insertion.
    pub deadtime: bool,
    /// Enable PWM synchronization (buffered CnV updates).
    pub sync: bool,
    /// Enable fault control for this pair.
    pub fault: bool,
}

/// A combined channel pair handle.
///
/// In combine mode, CnV (even) sets the rising edge and C(n+1)V (odd)
/// sets the falling edge. The pulse width is |C(n+1)V − CnV|.
pub struct FtmChannelPair<FTM, const PAIR: u8> {
    _ftm: PhantomData<FTM>,
}

impl<FTM: FtmInstance, const PAIR: u8> FtmChannelPair<FTM, PAIR> {
    /// Set the rising edge position (even channel CnV).
    pub fn set_rising_edge(&mut self, value: u16);

    /// Set the falling edge position (odd channel C(n+1)V).
    pub fn set_falling_edge(&mut self, value: u16);

    /// Set both edges atomically (writes both, then triggers sync if enabled).
    pub fn set_edges(&mut self, rising: u16, falling: u16);

    /// Set duty cycle as a simple pulse width.
    /// Rising edge at `start`, falling edge at `start + width`.
    pub fn set_pulse(&mut self, start: u16, width: u16);
}

impl<FTM: FtmInstance> FtmTimer<FTM, Enhanced> {
    /// Configure a channel pair for combine mode.
    ///
    /// Consumes the two individual `FtmChannel` handles and returns
    /// a `FtmChannelPair`. The even channel's pin carries the output;
    /// if complementary mode is enabled, the odd channel's pin carries
    /// the inverted output.
    pub fn combine(
        &mut self,
        even: FtmChannel<FTM, EVEN>,
        odd: FtmChannel<FTM, ODD>,
        config: CombineConfig,
    ) -> FtmChannelPair<FTM, PAIR>;
}
```

### The Channel Consumption Problem

`combine()` should consume the two individual channels to prevent aliasing. But const generics make this tricky — we need to enforce that EVEN and ODD are a valid pair (0+1, 2+3, etc.) at compile time. Options:

- **Macro-generated pairs:** Generate `combine_01()`, `combine_23()`, etc. as concrete methods. Less generic, but type-safe and simple.
- **Sealed trait with pair validation:** A `ValidPair<const EVEN: u8, const ODD: u8>` trait implemented only for valid pairs. Requires nightly features or workarounds.
- **Runtime check:** Accept any two channels, panic if not a valid pair. Simplest but not zero-cost.

**Recommendation:** Macro-generated concrete methods. There are only 4 pairs on FTM0 and 1 pair on FTM1/FTM2. This matches how stm32-hal2 handles it.

```rust
impl Ftm0PartsEnhanced {
    pub fn combine_01(
        &mut self,
        config: CombineConfig,
    ) -> FtmChannelPair<Ftm0, 0>;
    // ch0 and ch1 fields set to None or moved out
}
```

Or better — the Parts struct provides pairs directly:

```rust
pub struct Ftm0PartsEnhanced {
    pub timer: FtmTimer<Ftm0, Enhanced>,
    pub pair01: FtmChannelPair<Ftm0, 0>,  // or individual channels
    pub pair23: FtmChannelPair<Ftm0, 1>,
    pub pair45: FtmChannelPair<Ftm0, 2>,
    pub pair67: FtmChannelPair<Ftm0, 3>,
}
```

This is the imxrt-hal approach — enhanced split returns pairs, not individual channels. Individual channels can still be extracted from pairs if not using combine mode.

### Register Changes
- `MODE.FTMEN = 1`
- `COMBINE` register: COMBINEm, COMPm, DTENm, SYNCENm, FAULTENm per pair
- ELSnB:ELSnA on even channel controls polarity
- Write-protected

---

## Phase 4: Dead Time Insertion

**Complexity:** Low (given Phase 3 is done) — single register, only meaningful in combine+complementary mode.

Dead time prevents both outputs in a complementary pair from being active simultaneously. Duration = DTPS × DTVAL system clock cycles.

### API Design

```rust
/// Dead time configuration.
pub struct DeadtimeConfig {
    /// Prescaler: 1, 4, or 16.
    pub prescaler: DeadtimePrescaler,
    /// Count value (0-63). 0 disables dead time.
    pub value: u8,
}

pub enum DeadtimePrescaler {
    Div1,
    Div4,
    Div16,
}

impl<FTM: FtmInstance> FtmTimer<FTM, Enhanced> {
    /// Set the dead time duration.
    ///
    /// Dead time applies to all channel pairs that have DTENm=1 in their
    /// CombineConfig. The DEADTIME register is shared across all pairs.
    pub fn set_deadtime(&mut self, config: DeadtimeConfig);
}
```

### Register Changes
- `DEADTIME.DTPS[1:0]`: prescaler (0x=÷1, 10=÷4, 11=÷16)
- `DEADTIME.DTVAL[5:0]`: count value (0-63)
- Write-protected

---

## Phase 5: Enhanced Sync Mode

**Complexity:** High — this is the most complex feature, touching multiple registers and fundamentally changing how CnV/MOD updates work.

In enhanced sync mode (`SYNCONF.SYNCMODE=1`), writes to MOD, CNTIN, and CnV go to write buffers. They only take effect when a sync trigger fires AND the counter reaches a loading point (CNTMIN or CNTMAX). This enables coherent multi-channel updates — critical for motor control and DMA-driven waveform generation.

### API Design

```rust
/// Sync trigger source configuration.
pub struct SyncConfig {
    /// Software trigger updates MOD/CNTIN/CnV buffers.
    pub sw_write_buf: bool,
    /// Software trigger resets counter.
    pub sw_reset_counter: bool,
    /// Hardware trigger updates MOD/CNTIN/CnV buffers.
    pub hw_write_buf: bool,
    /// Hardware trigger resets counter.
    pub hw_reset_counter: bool,
    /// Loading point: counter minimum (CNT=CNTIN).
    pub load_at_min: bool,
    /// Loading point: counter maximum (CNT=MOD).
    pub load_at_max: bool,
}

impl<FTM: FtmInstance> FtmTimer<FTM, Enhanced> {
    /// Configure enhanced PWM synchronization.
    ///
    /// When enabled, writes to MOD, CNTIN, and CnV (for pairs with
    /// SYNCENm=1) go to write buffers. Call `sync_trigger()` to commit
    /// the buffered values at the next loading point.
    pub fn configure_sync(&mut self, config: SyncConfig);

    /// Issue a software sync trigger.
    ///
    /// Buffered MOD/CNTIN/CnV values will take effect at the next
    /// loading point (CNTMIN or CNTMAX, per SyncConfig).
    pub fn sync_trigger(&mut self);

    /// Load buffered values using PWMLOAD.
    ///
    /// Alternative to full sync: writes LDOK=1, which loads MOD/CNTIN/CnV
    /// at the next counter wrap. Simpler than SYNCONF for non-DMA use.
    pub fn load_ok(&mut self);
}
```

### DMA Coherency Pattern

The killer use case for enhanced sync: DMA writes multiple CnV values, then a final DMA transfer writes SYNC.SWSYNC=1 to commit them all atomically.

```rust
// DMA scatter-gather chain:
// Transfer 1: Write C0V (pair 0 rising edge)
// Transfer 2: Write C1V (pair 0 falling edge)
// Transfer 3: Write C2V (pair 1 rising edge)
// Transfer 4: Write C3V (pair 1 falling edge)
// Transfer 5: Write SYNC = 0x80 (SWSYNC=1, triggers atomic load)
//
// All CnV values take effect simultaneously at next counter boundary.
```

This eliminates the glitching that occurs when updating multiple channels non-atomically.

### Register Changes
- `SYNCONF`: SYNCMODE, SWWRBUF, HWWRBUF, SWRSTCNT, HWRSTCNT, CNTINC
- `SYNC`: SWSYNC (software trigger), CNTMIN, CNTMAX (loading points)
- `PWMLOAD`: LDOK, CHnSEL (alternative loading mechanism)

---

## Phase 6: Fault Protection

**Complexity:** Medium — several registers but straightforward logic.

Fault inputs (FLT0-FLT3) force PWM outputs to safe values (inactive state per POL). Used in motor control to protect against overcurrent, overtemperature, etc.

### API Design

```rust
pub struct FaultConfig {
    /// Fault mode: which channels are affected.
    pub mode: FaultMode,
    /// Enable fault interrupt.
    pub interrupt: bool,
}

pub enum FaultMode {
    /// Fault control disabled.
    Disabled,
    /// Fault affects even channels only, manual clearing.
    EvenManual,
    /// Fault affects all channels, manual clearing.
    AllManual,
    /// Fault affects all channels, automatic clearing.
    AllAutomatic,
}

pub struct FaultInputConfig {
    /// Active polarity: true = active-low, false = active-high.
    pub active_low: bool,
    /// Filter depth (0 = disabled, 1-15 = N system clocks).
    pub filter: u8,
}

impl<FTM: FtmInstance> FtmTimer<FTM, Enhanced> {
    /// Configure fault protection mode.
    pub fn configure_fault(&mut self, config: FaultConfig);

    /// Configure a fault input (0-3).
    pub fn configure_fault_input(&mut self, input: u8, config: FaultInputConfig);

    /// Check if a fault is currently active.
    pub fn has_fault(&self) -> bool;

    /// Clear the fault flag (for manual clearing modes).
    pub fn clear_fault(&mut self);
}
```

### Register Changes
- `MODE.FAULTM[1:0]`: fault mode
- `MODE.FAULTIE`: fault interrupt enable
- `FLTCTRL`: FAULTnEN, FFLTRnEN, FFVAL per input
- `FLTPOL`: FLTnPOL per input
- `FMS`: FAULTF, FAULTFn flags, FAULTIN real-time status
- `COMBINE.FAULTENm`: per-pair fault enable
- Write-protected

---

## Phase 7: Deprecate Standalone OutputCompare / InputCapture

**Complexity:** Low — documentation and deprecation attributes.

Once the split API is established and tests are migrated, mark the standalone types as deprecated.

```rust
#[deprecated(since = "0.2.0", note = "Use FtmChannel::set_output_compare() via FtmExt::split()")]
pub struct OutputCompare<FTM, const CH: u8> { ... }

#[deprecated(since = "0.2.0", note = "Use FtmChannel::set_input_capture() via FtmExt::split()")]
pub struct InputCapture<FTM, const CH: u8> { ... }
```

Remove in a future major version.

---

## Implementation Order Summary

| Phase | Feature | FTMEN | Complexity | Dependencies |
|-------|---------|-------|------------|--------------|
| 1 | Center-aligned PWM | 0 | Low | None |
| 2 | Output polarity + FTMEN typestate | 1 | Medium | FTMEN=1 design |
| 3 | Combine + complementary | 1 | Medium | Phase 2 |
| 4 | Dead time insertion | 1 | Low | Phase 3 |
| 5 | Enhanced sync | 1 | High | Phase 2 |
| 6 | Fault protection | 1 | Medium | Phase 2 |
| 7 | Deprecate standalone OC/IC | — | Low | Split API stable |

Phases 3-6 can be done in any order after Phase 2 establishes the FTMEN=1 infrastructure. Phase 5 (enhanced sync) is the most valuable for DMA-driven motor control and LED driving, so it may be worth prioritizing after Phase 3.

## Target Use Cases

**SK6812/WS2812 LED driving (current):**
- Edge-aligned PWM + DMA on single channel (Phase 0 — already works)
- Enhanced sync would allow atomic duty updates (Phase 5)

**Motor control (H-bridge, 3-phase BLDC):**
- Combine + complementary for high/low side gate drivers (Phase 3)
- Dead time to prevent shoot-through (Phase 4)
- Fault protection for overcurrent shutdown (Phase 6)
- Enhanced sync for coherent multi-phase updates (Phase 5)

**Servo / analog signal generation:**
- Center-aligned PWM for lower harmonic content (Phase 1)
- Output polarity for inverted signals (Phase 2)
