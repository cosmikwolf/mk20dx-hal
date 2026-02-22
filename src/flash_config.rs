/// Flash Configuration Field (16 bytes at 0x400-0x40F)
///
/// This must be placed at exactly address 0x400 in flash to configure
/// the chip's flash security, protection, and boot options.
///
/// The linker script defines a `.flashconfig` section at 0x400.
#[link_section = ".flashconfig"]
#[used]
#[no_mangle]
pub static FLASH_CONFIG: [u8; 16] = [
    0xFF, 0xFF, 0xFF, 0xFF, // Backdoor comparison key (bytes 0-3)
    0xFF, 0xFF, 0xFF, 0xFF, // Backdoor comparison key (bytes 4-7)
    0xFF, 0xFF, 0xFF, 0xFF, // Program flash protection (FPROT0-3)
    0xFE, // FSEC: Unsecured (SEC=10), Backdoor key disabled (KEYEN=01)
    0xFF, // FOPT: All options at default
    0xFF, // Reserved
    0xFF, // Reserved
];
