pub use embedded_hal::delay::DelayNs as _;
pub use embedded_hal::digital::InputPin as _;
pub use embedded_hal::digital::OutputPin as _;
pub use embedded_hal::digital::StatefulOutputPin as _;

pub use crate::adc::AdcExt as _;
pub use crate::clocks::McgExt as _;
pub use crate::cmp::CmpExt as _;
#[cfg(feature = "mk20d7")]
pub use crate::dac::DacExt as _;
pub use crate::dma::DmaExt as _;
pub use crate::flash::FlashExt as _;
pub use crate::gpio::GpioExt as _;
pub use crate::i2c::I2cExt as _;
pub use crate::pwm::FtmExt as _;
pub use crate::rtc::RtcExt as _;
pub use crate::time::U32Ext as _;
pub use crate::spi::SpiExt as _;
pub use crate::timer::PitExt as _;
pub use crate::uart::UartExt as _;
pub use crate::usb::UsbBusExt as _;
pub use crate::watchdog::WdogExt as _;
