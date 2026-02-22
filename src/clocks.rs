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
    /// Consumes the OSC peripheral. Borrows SIM since other modules also need it.
    pub fn freeze(self, osc: OscPeripheral, sim: &pac::Sim) -> Clocks {
        let mcg = self.mcg;

        // -- Step 0: Set SIM clock dividers before switching clock source --
        // This ensures bus/flash clocks stay within spec during the transition.
        #[cfg(feature = "mk20d7")]
        sim.clkdiv1().write(|w| {
            w.outdiv1()._0000()  // ÷1 → 72 MHz core
             .outdiv2()._0001()  // ÷2 → 36 MHz bus
             .outdiv4()._0010()  // ÷3 → 24 MHz flash
        });
        #[cfg(feature = "mk20d5")]
        sim.clkdiv1().write(|w| {
            w.outdiv1()._0000()  // ÷1 → 48 MHz core
             .outdiv2()._0000()  // ÷1 → 48 MHz bus
             .outdiv4()._0001()  // ÷2 → 24 MHz flash
        });

        // -- Step 1: Enable external oscillator --
        // Teensy uses a 16 MHz crystal with ~10 pF load capacitors (SC8P + SC2P).
        osc.cr().write(|w| {
            w.erclken()._1()  // Enable external reference clock output
             .sc8p()._1()     // 8 pF load cap
             .sc2p()._1()     // 2 pF load cap
        });

        // -- Step 2: FEI → FBE (FLL Bypassed External) --
        // Configure MCG C2 for very-high-frequency-range crystal oscillator
        unsafe {
            mcg.c2().write(|w| {
                w.range0().bits(2)   // Very high frequency range (8-32 MHz)
                 .erefs0()._1()      // Oscillator requested (not external clock)
                 .hgo0()._0()        // Low-power oscillator mode
            });
        }

        // Switch to external reference clock, set FRDIV for 16 MHz ÷ 512 = 31.25 kHz
        mcg.c1().write(|w| {
            w.clks().external()  // Select external reference clock
             .frdiv()._100()     // Divide by 512 (for high-range oscillator)
             .irefs()._0()       // External reference
        });

        // Wait for oscillator to initialize
        while mcg.s().read().oscinit0().bit_is_clear() {}
        // Wait for clock source to switch to external reference
        while !mcg.s().read().clkst().is_10() {}
        // Wait for FLL reference to switch to external
        while mcg.s().read().irefst().is_1() {}

        // -- Step 3: FBE → PBE (PLL Bypassed External) --
        // Configure PLL: reference_freq = 16 MHz ÷ (PRDIV + 1)
        // PLL output = reference_freq × (VDIV + 24)
        #[cfg(feature = "mk20d7")]
        {
            // PRDIV=7 → 16/(7+1)=2 MHz, VDIV=12 → 2×(12+24)=72 MHz
            unsafe {
                mcg.c5().write(|w| w.prdiv0().bits(7));   // ÷8 → 2 MHz
                mcg.c6().write(|w| w.vdiv0().bits(12)     // ×36 → 72 MHz
                                    .plls()._1());         // Select PLL
            }
        }
        #[cfg(feature = "mk20d5")]
        {
            // PRDIV=7 → 16/(7+1)=2 MHz, VDIV=0 → 2×(0+24)=48 MHz
            unsafe {
                mcg.c5().write(|w| w.prdiv0().bits(7));   // ÷8 → 2 MHz
                mcg.c6().write(|w| w.vdiv0().bits(0)      // ×24 → 48 MHz
                                    .plls()._1());         // Select PLL
            }
        }

        // Wait for PLL to be selected as PLLS clock source
        while !mcg.s().read().pllst().is_1() {}
        // Wait for PLL to lock
        while !mcg.s().read().lock0().is_1() {}

        // -- Step 4: PBE → PEE (PLL Engaged External) --
        // Switch CLKS to FLL/PLL output (which is PLL since PLLS=1)
        mcg.c1().modify(|_, w| w.clks().fll_pll());

        // Wait for clock source to switch to PLL
        while !mcg.s().read().clkst().is_11() {}

        // -- Done: return frozen clock frequencies --
        #[cfg(feature = "mk20d7")]
        let clocks = Clocks {
            core_clk: Hertz::from_raw(72_000_000),
            bus_clk: Hertz::from_raw(36_000_000),
            flash_clk: Hertz::from_raw(24_000_000),
        };
        #[cfg(feature = "mk20d5")]
        let clocks = Clocks {
            core_clk: Hertz::from_raw(48_000_000),
            bus_clk: Hertz::from_raw(48_000_000),
            flash_clk: Hertz::from_raw(24_000_000),
        };

        clocks
    }
}
