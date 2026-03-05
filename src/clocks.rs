use crate::pac;
use crate::time::Hertz;

/// Frozen clock frequencies.
///
/// This type is returned by [`Mcg::freeze`] after the clock tree has been
/// set up. It records the resulting frequencies and serves as proof
/// that clocks have been configured (required by most peripheral drivers).
#[derive(Clone, Copy, Debug)]
pub struct Clocks {
    core_clk: Hertz,
    bus_clk: Hertz,
    flash_clk: Hertz,
}

impl Clocks {
    /// Core / system clock frequency (OUTDIV1).
    pub fn core_clk(&self) -> Hertz {
        self.core_clk
    }

    /// Bus clock frequency (OUTDIV2). Used by most peripherals.
    pub fn bus_clk(&self) -> Hertz {
        self.bus_clk
    }

    /// Flash clock frequency (OUTDIV4).
    pub fn flash_clk(&self) -> Hertz {
        self.flash_clk
    }
}

/// OSC peripheral type alias — differs between chip variants.
#[cfg(feature = "mk20d7")]
pub type OscPeripheral = pac::Osc;
#[cfg(feature = "mk20d5")]
pub type OscPeripheral = pac::Osc0;

/// Clock speed preset for MK20D7 (Teensy 3.1/3.2).
///
/// All presets assume a 16 MHz crystal (standard on Teensy boards).
/// `Mhz96` and `Mhz120` exceed the datasheet-rated maximum of 72 MHz
/// but are widely used and tested in the Teensy community.
#[cfg(feature = "mk20d7")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ClockSpeed {
    /// 72 MHz core, 36 MHz bus, 24 MHz flash — datasheet rated maximum.
    Mhz72,
    /// 96 MHz core, 48 MHz bus, 24 MHz flash — overclock, widely used.
    Mhz96,
    /// 120 MHz core, 60 MHz bus, 24 MHz flash — overclock, Teensy default.
    Mhz120,
}

/// Extension trait for the MCG peripheral.
///
/// Provides `constrain()` to convert the PAC MCG into the HAL's
/// clock configuration builder.
pub trait McgExt {
    /// Consume the PAC MCG peripheral and return the HAL wrapper.
    fn constrain(self) -> Mcg;
}

impl McgExt for pac::Mcg {
    fn constrain(self) -> Mcg {
        Mcg { mcg: self }
    }
}

/// MCG wrapper that provides clock configuration.
///
/// Created by calling [`McgExt::constrain`] on the PAC MCG peripheral.
/// Call [`freeze`](Mcg::freeze) to configure the clock tree and obtain
/// the frozen [`Clocks`] token.
pub struct Mcg {
    mcg: pac::Mcg,
}

impl Mcg {
    /// Configure the clock tree for the Teensy's 16 MHz crystal.
    ///
    /// Transitions the MCG through FEI → FBE → PBE → PEE to achieve
    /// PLL-engaged-external mode. Configures SIM clock dividers for
    /// the target frequencies.
    ///
    /// # Target Frequencies
    ///
    /// - **mk20d7** (Teensy 3.1/3.2): 72 MHz core, 36 MHz bus, 24 MHz flash
    /// - **mk20d5** (Teensy 3.0): 48 MHz core, 48 MHz bus, 24 MHz flash
    ///
    /// For mk20d7, this is equivalent to `freeze_at(ClockSpeed::Mhz72, ..)`.
    /// Use [`freeze_at`](Mcg::freeze_at) to select 96 or 120 MHz overclock
    /// presets on Teensy 3.1/3.2.
    ///
    /// Consumes the OSC peripheral. Borrows SIM since other modules also need it.
    pub fn freeze(self, osc: OscPeripheral, sim: &pac::Sim) -> Clocks {
        #[cfg(feature = "mk20d7")]
        return self.freeze_at(ClockSpeed::Mhz72, osc, sim);

        #[cfg(feature = "mk20d5")]
        {
            let mcg = self.mcg;

            // -- Step 0: Set SIM clock dividers before switching clock source --
            sim.clkdiv1().write(|w| {
                w.outdiv1().div1()  // ÷1 → 48 MHz core
                 .outdiv2().div1()  // ÷1 → 48 MHz bus
                 .outdiv4().div2()  // ÷2 → 24 MHz flash
            });

            // -- Step 1: Enable external oscillator --
            osc.cr().write(|w| {
                w.erclken()._1()  // Enable external reference clock output
                 .sc8p()._1()     // 8 pF load cap
                 .sc2p()._1()     // 2 pF load cap
            });

            // -- Step 2: FEI → FBE (FLL Bypassed External) --
            // SAFETY: range0 is a 2-bit field; value 2 selects very high frequency range.
            unsafe {
                mcg.c2().write(|w| {
                    w.range0().bits(2)      // Very high frequency range (8-32 MHz)
                     .erefs0().oscillator() // Oscillator requested (not external clock)
                     .hgo0().low_power()    // Low-power oscillator mode
                });
            }

            mcg.c1().write(|w| {
                // SAFETY: frdiv is a 3-bit field; 4 = 0b100 selects ÷512 for high-range oscillator.
                let w = unsafe { w.frdiv().bits(4) };
                w.clks().external()  // Select external reference clock
                 .irefs().external() // External FLL reference
            });

            while mcg.s().read().oscinit0().bit_is_clear() {}
            while !mcg.s().read().clkst().is_external() {}
            while mcg.s().read().irefst().is_internal() {}

            // -- Step 3: FBE → PBE (PLL Bypassed External) --
            // PRDIV=7 → 16/(7+1)=2 MHz, VDIV=0 → 2×(0+24)=48 MHz
            // SAFETY: prdiv0 is a 5-bit field (value 7 fits), vdiv0 is a 5-bit
            // field (value 0 fits).
            unsafe {
                mcg.c5().write(|w| w.prdiv0().bits(7));   // ÷8 → 2 MHz
                mcg.c6().write(|w| w.vdiv0().bits(0)      // ×24 → 48 MHz
                                    .plls()._1());         // Select PLL (no semantic enum for PLLS)
            }

            while !mcg.s().read().pllst().is_pll() {}
            while !mcg.s().read().lock0().is_locked() {}

            // -- Step 4: PBE → PEE (PLL Engaged External) --
            mcg.c1().modify(|_, w| w.clks().fll_pll());
            while !mcg.s().read().clkst().is_pll() {}

            Clocks {
                core_clk: Hertz::from_raw(48_000_000),
                bus_clk: Hertz::from_raw(48_000_000),
                flash_clk: Hertz::from_raw(24_000_000),
            }
        }
    }

    /// Configure the clock tree at a specific speed preset (mk20d7 only).
    ///
    /// Transitions the MCG through FEI → FBE → PBE → PEE to achieve
    /// PLL-engaged-external mode. The PLL dividers and SIM clock dividers
    /// are set according to the selected [`ClockSpeed`] preset.
    ///
    /// All presets assume a 16 MHz crystal (standard on Teensy boards).
    ///
    /// | Speed | Core | Bus | Flash | PRDIV | VDIV |
    /// |-------|------|-----|-------|-------|------|
    /// | `Mhz72` | 72 MHz | 36 MHz | 24 MHz | ÷8 | ×36 |
    /// | `Mhz96` | 96 MHz | 48 MHz | 24 MHz | ÷4 | ×24 |
    /// | `Mhz120` | 120 MHz | 60 MHz | 24 MHz | ÷4 | ×30 |
    ///
    /// Consumes the OSC peripheral. Borrows SIM since other modules also need it.
    #[cfg(feature = "mk20d7")]
    pub fn freeze_at(self, speed: ClockSpeed, osc: OscPeripheral, sim: &pac::Sim) -> Clocks {
        let mcg = self.mcg;

        // PLL parameters and clock dividers for each speed preset.
        // PLL ref = 16 MHz / (prdiv + 1), PLL out = ref × (vdiv + 24)
        let (prdiv, vdiv, outdiv2, outdiv4, core_hz, bus_hz, flash_hz) = match speed {
            // 16/8=2 MHz × 36 = 72 MHz, ÷2=36 bus, ÷3=24 flash
            ClockSpeed::Mhz72 => (7, 12, 1, 2, 72_000_000, 36_000_000, 24_000_000),
            // 16/4=4 MHz × 24 = 96 MHz, ÷2=48 bus, ÷4=24 flash
            ClockSpeed::Mhz96 => (3, 0, 1, 3, 96_000_000, 48_000_000, 24_000_000),
            // 16/4=4 MHz × 30 = 120 MHz, ÷2=60 bus, ÷5=24 flash
            ClockSpeed::Mhz120 => (3, 6, 1, 4, 120_000_000, 60_000_000, 24_000_000),
        };

        // -- Step 0: Set SIM clock dividers before switching clock source --
        // This ensures bus/flash clocks stay within spec during the transition.
        // SAFETY: CLKDIV1 fields are 4 bits each; max value here is 4 (fits).
        unsafe {
            sim.clkdiv1().write(|w| {
                w.outdiv1().bits(0)       // ÷1 → core = PLL out
                 .outdiv2().bits(outdiv2)  // ÷(outdiv2+1) → bus
                 .outdiv4().bits(outdiv4)  // ÷(outdiv4+1) → flash
            });
        }

        // -- Step 1: Enable external oscillator --
        // Teensy uses a 16 MHz crystal with ~10 pF load capacitors (SC8P + SC2P).
        osc.cr().write(|w| {
            w.erclken()._1()  // Enable external reference clock output
             .sc8p()._1()     // 8 pF load cap
             .sc2p()._1()     // 2 pF load cap
        });

        // -- Step 2: FEI → FBE (FLL Bypassed External) --
        // Configure MCG C2 for very-high-frequency-range crystal oscillator
        // SAFETY: range0 is a 2-bit field; value 2 selects very high frequency range.
        unsafe {
            mcg.c2().write(|w| {
                w.range0().bits(2)      // Very high frequency range (8-32 MHz)
                 .erefs0().oscillator() // Oscillator requested (not external clock)
                 .hgo0().low_power()    // Low-power oscillator mode
            });
        }

        // Switch to external reference clock, set FRDIV for 16 MHz ÷ 512 = 31.25 kHz
        mcg.c1().write(|w| {
            // SAFETY: frdiv is a 3-bit field; 4 = 0b100 selects ÷512 for high-range oscillator.
            let w = unsafe { w.frdiv().bits(4) };
            w.clks().external()  // Select external reference clock
             .irefs().external() // External FLL reference
        });

        // Wait for oscillator to initialize
        while mcg.s().read().oscinit0().bit_is_clear() {}
        // Wait for clock source to switch to external reference
        while !mcg.s().read().clkst().is_external() {}
        // Wait for FLL reference to switch to external
        while mcg.s().read().irefst().is_internal() {}

        // -- Step 3: FBE → PBE (PLL Bypassed External) --
        // Configure PLL: reference_freq = 16 MHz ÷ (PRDIV + 1)
        // PLL output = reference_freq × (VDIV + 24)
        // SAFETY: prdiv0 is a 5-bit field, vdiv0 is a 5-bit field.
        // All values from the match above fit within 5 bits.
        unsafe {
            mcg.c5().write(|w| w.prdiv0().bits(prdiv));
            mcg.c6().write(|w| w.vdiv0().bits(vdiv)
                                .plls()._1());  // Select PLL (no semantic enum for PLLS)
        }

        // Wait for PLL to be selected as PLLS clock source
        while !mcg.s().read().pllst().is_pll() {}
        // Wait for PLL to lock
        while !mcg.s().read().lock0().is_locked() {}

        // -- Step 4: PBE → PEE (PLL Engaged External) --
        // Switch CLKS to FLL/PLL output (which is PLL since PLLS=1)
        mcg.c1().modify(|_, w| w.clks().fll_pll());

        // Wait for clock source to switch to PLL
        while !mcg.s().read().clkst().is_pll() {}

        Clocks {
            core_clk: Hertz::from_raw(core_hz),
            bus_clk: Hertz::from_raw(bus_hz),
            flash_clk: Hertz::from_raw(flash_hz),
        }
    }
}

/// Saved PEE-mode MCG state for restoring after BLPI exit.
///
/// Created by [`enter_blpi`](Clocks::enter_blpi), consumed by
/// [`exit_blpi`](Clocks::exit_blpi).
pub struct PeeState {
    core_clk: Hertz,
    bus_clk: Hertz,
    flash_clk: Hertz,
    clkdiv1: u32,
}

/// Reduced-frequency clocks for VLPR mode (BLPI).
///
/// In BLPI mode the MCG outputs the fast internal reference clock
/// (up to 4 MHz). The `Clocks` token is consumed to prevent
/// peripheral drivers from using stale frequency values.
pub struct BlpiClocks {
    core_clk: Hertz,
    bus_clk: Hertz,
    flash_clk: Hertz,
}

impl BlpiClocks {
    /// Core clock frequency in BLPI mode.
    pub fn core_clk(&self) -> Hertz {
        self.core_clk
    }

    /// Bus clock frequency in BLPI mode.
    pub fn bus_clk(&self) -> Hertz {
        self.bus_clk
    }

    /// Flash clock frequency in BLPI mode.
    pub fn flash_clk(&self) -> Hertz {
        self.flash_clk
    }
}

impl Clocks {
    /// Transition MCG from PEE to BLPI for VLPR mode entry.
    ///
    /// MCG transition path: PEE → PBE → FBE → FBI → BLPI
    ///
    /// After this call, the core runs from the fast internal reference
    /// clock (~4 MHz with FCRDIV=0, or divided down). SIM dividers are
    /// adjusted so all clocks stay within VLPR limits.
    ///
    /// # VLPR Clock Limits (MK20DX)
    /// - Core: max 4 MHz (mk20d7) / 2 MHz (mk20d5)
    /// - Bus: max 1 MHz
    /// - Flash: max 1 MHz
    ///
    /// Returns `BlpiClocks` (reduced-frequency token) and `PeeState`
    /// (saved state for restoring PEE mode later).
    pub fn enter_blpi(self, sim: &pac::Sim) -> (BlpiClocks, PeeState) {
        let mcg = Self::mcg_regs();

        // Save current frequencies and SIM dividers for restoration
        let saved = PeeState {
            core_clk: self.core_clk,
            bus_clk: self.bus_clk,
            flash_clk: self.flash_clk,
            clkdiv1: sim.clkdiv1().read().bits(),
        };

        // -- Step 1: PEE → PBE --
        // Switch CLKS from FLL/PLL output to external reference
        mcg.c1().modify(|_, w| w.clks().external());
        // Wait for external clock source
        while !mcg.s().read().clkst().is_external() {}

        // -- Step 2: PBE → FBE --
        // Deselect PLL
        mcg.c6().modify(|_, w| w.plls()._0());
        // Wait for FLL to be selected
        while mcg.s().read().pllst().is_pll() {}

        // -- Step 3: FBE → FBI --
        // Switch to internal reference clock
        mcg.c1().modify(|_, w| w.clks().internal().irefs().internal());
        // Wait for internal reference
        while !mcg.s().read().clkst().is_internal() {}
        while !mcg.s().read().irefst().is_internal() {}

        // Select fast internal reference clock (~4 MHz undivided)
        mcg.c2().modify(|_, w| w.ircs().fast_irc());
        // Wait for fast IRC selected
        while !mcg.s().read().ircst().is_fast() {}

        // -- Step 4: FBI → BLPI --
        // Set LP bit to disable FLL in bypass mode
        mcg.c2().modify(|_, w| w.lp().fll_pll_disabled());

        // -- Step 5: Adjust SIM dividers for VLPR limits --
        // Fast IRC = 4 MHz. Need core ≤ 4 MHz, bus ≤ 1 MHz, flash ≤ 1 MHz
        sim.clkdiv1().write(|w| {
            w.outdiv1().div1()  // ÷1 → 4 MHz core
             .outdiv2().div4()  // ÷4 → 1 MHz bus
             .outdiv4().div4()  // ÷4 → 1 MHz flash
        });

        let blpi_clocks = BlpiClocks {
            core_clk: Hertz::from_raw(4_000_000),
            bus_clk: Hertz::from_raw(1_000_000),
            flash_clk: Hertz::from_raw(1_000_000),
        };

        (blpi_clocks, saved)
    }

    /// Transition MCG from BLPI back to PEE (normal run mode).
    ///
    /// MCG transition path: BLPI → FBI → FBE → PBE → PEE
    ///
    /// Restores clock dividers and frequencies to the values saved
    /// when [`enter_blpi`](Clocks::enter_blpi) was called.
    pub fn exit_blpi(_blpi: BlpiClocks, saved: PeeState, sim: &pac::Sim) -> Clocks {
        let mcg = Self::mcg_regs();

        // -- Step 1: BLPI → FBI --
        // Clear LP bit to re-enable FLL
        mcg.c2().modify(|_, w| w.lp().fll_pll_active());

        // -- Step 2: FBI → FBE --
        // Switch to external reference
        mcg.c1().modify(|_, w| w.clks().external().irefs().external());
        // Wait for external reference
        while !mcg.s().read().clkst().is_external() {}
        while mcg.s().read().irefst().is_internal() {}

        // -- Step 3: Restore SIM dividers before PLL engagement --
        // SAFETY: Restoring the exact CLKDIV1 value saved during enter_blpi().
        unsafe { sim.clkdiv1().write(|w| w.bits(saved.clkdiv1)) };

        // -- Step 4: FBE → PBE --
        // Re-enable PLL
        mcg.c6().modify(|_, w| w.plls()._1());
        // Wait for PLL selected
        while !mcg.s().read().pllst().is_pll() {}
        // Wait for PLL lock
        while !mcg.s().read().lock0().is_locked() {}

        // -- Step 5: PBE → PEE --
        mcg.c1().modify(|_, w| w.clks().fll_pll());
        // Wait for PLL output as system clock
        while !mcg.s().read().clkst().is_pll() {}

        Clocks {
            core_clk: saved.core_clk,
            bus_clk: saved.bus_clk,
            flash_clk: saved.flash_clk,
        }
    }

    /// Access the MCG register block after the PAC peripheral was consumed.
    ///
    /// This is safe because the PAC `Mcg` is consumed by `freeze()` and never
    /// reconstructed — no `steal()` call exists for MCG in this crate. Only
    /// `Clocks` methods access MCG afterwards, and `enter_blpi()` consumes
    /// `Clocks` while `exit_blpi()` consumes `BlpiClocks`, so at most one
    /// context can access MCG registers at a time. This guarantees exclusive
    /// access to MCG through the ownership chain.
    fn mcg_regs() -> &'static pac::mcg::RegisterBlock {
        // SAFETY: PAC Mcg was consumed by freeze() and is never reconstructed.
        // Exclusive access is enforced by the Clocks/BlpiClocks ownership chain.
        unsafe { &*pac::Mcg::PTR }
    }
}
