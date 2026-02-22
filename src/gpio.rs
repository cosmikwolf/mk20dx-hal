use core::marker::PhantomData;

use crate::pac;

// ----- Mode type-states -----

pub struct Input<PULL = Floating> {
    _pull: PhantomData<PULL>,
}
pub struct Output<TYPE = PushPull> {
    _type: PhantomData<TYPE>,
}
pub struct Alternate<const MUX: u8>;
pub struct Disabled;

pub struct Floating;
pub struct PullUp;
pub struct PullDown;
pub struct PushPull;
pub struct OpenDrain;

// ----- Pin type -----

/// A GPIO pin with compile-time port and pin number.
///
/// The `MODE` type parameter tracks the pin's current configuration
/// (input, output, alternate function, etc.) at compile time.
pub struct Pin<const PORT: char, const N: u8, MODE = Disabled> {
    _mode: PhantomData<MODE>,
}

impl<const PORT: char, const N: u8, MODE> Pin<PORT, N, MODE> {
    /// Get a reference to the PORT register block for this pin.
    fn port(&self) -> &pac::porta::RegisterBlock {
        unsafe { &*port_ptr(PORT) }
    }

    /// Get a reference to the GPIO register block for this pin.
    fn gpio(&self) -> &pac::pta::RegisterBlock {
        unsafe { &*gpio_ptr(PORT) }
    }

    /// Configure as floating input.
    pub fn into_floating_input(self) -> Pin<PORT, N, Input<Floating>> {
        let port = self.port();
        let gpio = self.gpio();
        // Set as input in GPIO direction register
        gpio.pddr().modify(|r, w| unsafe { w.bits(r.bits() & !(1 << N)) });
        // Set MUX to GPIO, disable pull
        port.pcr(N as usize).write(|w| {
            w.mux().gpio()
             .pe()._0() // Pull disabled
        });
        Pin { _mode: PhantomData }
    }

    /// Configure as input with pull-up.
    pub fn into_pull_up_input(self) -> Pin<PORT, N, Input<PullUp>> {
        let port = self.port();
        let gpio = self.gpio();
        gpio.pddr().modify(|r, w| unsafe { w.bits(r.bits() & !(1 << N)) });
        port.pcr(N as usize).write(|w| {
            w.mux().gpio()
             .pe()._1() // Pull enabled
             .ps()._1() // Pull-up
        });
        Pin { _mode: PhantomData }
    }

    /// Configure as input with pull-down.
    pub fn into_pull_down_input(self) -> Pin<PORT, N, Input<PullDown>> {
        let port = self.port();
        let gpio = self.gpio();
        gpio.pddr().modify(|r, w| unsafe { w.bits(r.bits() & !(1 << N)) });
        port.pcr(N as usize).write(|w| {
            w.mux().gpio()
             .pe()._1() // Pull enabled
             .ps()._0() // Pull-down
        });
        Pin { _mode: PhantomData }
    }

    /// Configure as push-pull output.
    pub fn into_push_pull_output(self) -> Pin<PORT, N, Output<PushPull>> {
        let port = self.port();
        let gpio = self.gpio();
        // Set as output in GPIO direction register
        gpio.pddr().modify(|r, w| unsafe { w.bits(r.bits() | (1 << N)) });
        // Set MUX to GPIO, disable open drain
        port.pcr(N as usize).write(|w| {
            w.mux().gpio()
             .ode()._0() // Open drain disabled
        });
        Pin { _mode: PhantomData }
    }

    /// Configure as open-drain output.
    pub fn into_open_drain_output(self) -> Pin<PORT, N, Output<OpenDrain>> {
        let port = self.port();
        let gpio = self.gpio();
        gpio.pddr().modify(|r, w| unsafe { w.bits(r.bits() | (1 << N)) });
        port.pcr(N as usize).write(|w| {
            w.mux().gpio()
             .ode()._1() // Open drain enabled
        });
        Pin { _mode: PhantomData }
    }

    /// Configure for an alternate function (peripheral mux).
    pub fn into_alternate<const MUX: u8>(self) -> Pin<PORT, N, Alternate<MUX>> {
        let port = self.port();
        port.pcr(N as usize).modify(|_, w| unsafe {
            w.mux().bits(MUX)
        });
        Pin { _mode: PhantomData }
    }
}

// ----- embedded-hal trait implementations -----

impl<const PORT: char, const N: u8, PULL> embedded_hal::digital::ErrorType
    for Pin<PORT, N, Input<PULL>>
{
    type Error = core::convert::Infallible;
}

impl<const PORT: char, const N: u8, TYPE> embedded_hal::digital::ErrorType
    for Pin<PORT, N, Output<TYPE>>
{
    type Error = core::convert::Infallible;
}

impl<const PORT: char, const N: u8, PULL> embedded_hal::digital::InputPin
    for Pin<PORT, N, Input<PULL>>
{
    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(self.gpio().pdir().read().bits() & (1 << N) != 0)
    }

    fn is_low(&mut self) -> Result<bool, Self::Error> {
        Ok(self.gpio().pdir().read().bits() & (1 << N) == 0)
    }
}

impl<const PORT: char, const N: u8, TYPE> embedded_hal::digital::OutputPin
    for Pin<PORT, N, Output<TYPE>>
{
    fn set_high(&mut self) -> Result<(), Self::Error> {
        // PSOR is write-only, set-on-write — atomic, no read-modify-write needed
        self.gpio().psor().write(|w| unsafe { w.bits(1 << N) });
        Ok(())
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        // PCOR is write-only, clear-on-write — atomic
        self.gpio().pcor().write(|w| unsafe { w.bits(1 << N) });
        Ok(())
    }
}

impl<const PORT: char, const N: u8, TYPE> embedded_hal::digital::StatefulOutputPin
    for Pin<PORT, N, Output<TYPE>>
{
    fn is_set_high(&mut self) -> Result<bool, Self::Error> {
        Ok(self.gpio().pdor().read().bits() & (1 << N) != 0)
    }

    fn is_set_low(&mut self) -> Result<bool, Self::Error> {
        Ok(self.gpio().pdor().read().bits() & (1 << N) == 0)
    }

    fn toggle(&mut self) -> Result<(), Self::Error> {
        // PTOR is write-only, toggle-on-write — atomic
        self.gpio().ptor().write(|w| unsafe { w.bits(1 << N) });
        Ok(())
    }
}

// ----- Port pointer lookup -----

const fn port_ptr(port: char) -> *const pac::porta::RegisterBlock {
    match port {
        'A' => pac::Porta::PTR,
        'B' => pac::Portb::PTR as *const pac::porta::RegisterBlock,
        'C' => pac::Portc::PTR as *const pac::porta::RegisterBlock,
        'D' => pac::Portd::PTR as *const pac::porta::RegisterBlock,
        'E' => pac::Porte::PTR as *const pac::porta::RegisterBlock,
        _ => panic!("Invalid GPIO port"),
    }
}

const fn gpio_ptr(port: char) -> *const pac::pta::RegisterBlock {
    match port {
        'A' => pac::Pta::PTR,
        'B' => pac::Ptb::PTR as *const pac::pta::RegisterBlock,
        'C' => pac::Ptc::PTR as *const pac::pta::RegisterBlock,
        'D' => pac::Ptd::PTR as *const pac::pta::RegisterBlock,
        'E' => pac::Pte::PTR as *const pac::pta::RegisterBlock,
        _ => panic!("Invalid GPIO port"),
    }
}

// ----- Extension trait for port splitting -----

/// Extension trait for splitting PAC PORT peripherals into individual pins.
///
/// This trait is implemented on each PAC PORT type (Porta, Portb, etc.)
/// and provides a `split()` method that consumes the PORT and GPIO
/// peripherals, enables the SIM clock gate, and returns a struct of
/// individually-owned pins.
pub trait GpioExt {
    /// The struct of individual pins returned by `split()`.
    type Pins;
    /// The GPIO peripheral type paired with this PORT.
    type Gpio;

    /// Split the port into individual pin types.
    ///
    /// Consumes both the PORT and GPIO peripherals. Borrows SIM to
    /// enable the port's clock gate in SCGC5.
    fn split(self, gpio: Self::Gpio, sim: &pac::Sim) -> Self::Pins;
}

// ----- Port splitting macro -----

macro_rules! gpio_port_impl {
    (
        $PORT:ty, $GPIO:ty, $PortPins:ident,
        $port_char:literal, $scgc_field:ident,
        [$(($field:ident, $pin_n:literal)),*]
    ) => {
        pub struct $PortPins {
            $(
                pub $field: Pin<$port_char, $pin_n, Disabled>,
            )*
        }

        impl GpioExt for $PORT {
            type Pins = $PortPins;
            type Gpio = $GPIO;

            fn split(self, _gpio: $GPIO, sim: &pac::Sim) -> $PortPins {
                sim.scgc5().modify(|_, w| w.$scgc_field()._1());

                $PortPins {
                    $(
                        $field: Pin { _mode: PhantomData },
                    )*
                }
            }
        }
    };
}

gpio_port_impl!(
    pac::Porta, pac::Pta, PortAPins, 'A', porta,
    [
        (pa0, 0), (pa1, 1), (pa2, 2), (pa3, 3), (pa4, 4), (pa5, 5),
        (pa6, 6), (pa7, 7), (pa8, 8), (pa9, 9), (pa10, 10), (pa11, 11),
        (pa12, 12), (pa13, 13), (pa14, 14), (pa15, 15), (pa16, 16),
        (pa17, 17), (pa18, 18), (pa19, 19), (pa20, 20), (pa21, 21),
        (pa22, 22), (pa23, 23), (pa24, 24), (pa25, 25), (pa26, 26),
        (pa27, 27), (pa28, 28), (pa29, 29), (pa30, 30), (pa31, 31)
    ]
);

gpio_port_impl!(
    pac::Portb, pac::Ptb, PortBPins, 'B', portb,
    [
        (pb0, 0), (pb1, 1), (pb2, 2), (pb3, 3), (pb4, 4), (pb5, 5),
        (pb6, 6), (pb7, 7), (pb8, 8), (pb9, 9), (pb10, 10), (pb11, 11),
        (pb12, 12), (pb13, 13), (pb14, 14), (pb15, 15), (pb16, 16),
        (pb17, 17), (pb18, 18), (pb19, 19), (pb20, 20), (pb21, 21),
        (pb22, 22), (pb23, 23), (pb24, 24), (pb25, 25), (pb26, 26),
        (pb27, 27), (pb28, 28), (pb29, 29), (pb30, 30), (pb31, 31)
    ]
);

gpio_port_impl!(
    pac::Portc, pac::Ptc, PortCPins, 'C', portc,
    [
        (pc0, 0), (pc1, 1), (pc2, 2), (pc3, 3), (pc4, 4), (pc5, 5),
        (pc6, 6), (pc7, 7), (pc8, 8), (pc9, 9), (pc10, 10), (pc11, 11),
        (pc12, 12), (pc13, 13), (pc14, 14), (pc15, 15), (pc16, 16),
        (pc17, 17), (pc18, 18), (pc19, 19), (pc20, 20), (pc21, 21),
        (pc22, 22), (pc23, 23), (pc24, 24), (pc25, 25), (pc26, 26),
        (pc27, 27), (pc28, 28), (pc29, 29), (pc30, 30), (pc31, 31)
    ]
);

gpio_port_impl!(
    pac::Portd, pac::Ptd, PortDPins, 'D', portd,
    [
        (pd0, 0), (pd1, 1), (pd2, 2), (pd3, 3), (pd4, 4), (pd5, 5),
        (pd6, 6), (pd7, 7), (pd8, 8), (pd9, 9), (pd10, 10), (pd11, 11),
        (pd12, 12), (pd13, 13), (pd14, 14), (pd15, 15), (pd16, 16),
        (pd17, 17), (pd18, 18), (pd19, 19), (pd20, 20), (pd21, 21),
        (pd22, 22), (pd23, 23), (pd24, 24), (pd25, 25), (pd26, 26),
        (pd27, 27), (pd28, 28), (pd29, 29), (pd30, 30), (pd31, 31)
    ]
);

gpio_port_impl!(
    pac::Porte, pac::Pte, PortEPins, 'E', porte,
    [
        (pe0, 0), (pe1, 1), (pe2, 2), (pe3, 3), (pe4, 4), (pe5, 5),
        (pe6, 6), (pe7, 7), (pe8, 8), (pe9, 9), (pe10, 10), (pe11, 11),
        (pe12, 12), (pe13, 13), (pe14, 14), (pe15, 15), (pe16, 16),
        (pe17, 17), (pe18, 18), (pe19, 19), (pe20, 20), (pe21, 21),
        (pe22, 22), (pe23, 23), (pe24, 24), (pe25, 25), (pe26, 26),
        (pe27, 27), (pe28, 28), (pe29, 29), (pe30, 30), (pe31, 31)
    ]
);
