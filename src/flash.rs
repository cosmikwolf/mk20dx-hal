//! Flash memory (FTFL) driver for MK20DX128/MK20DX256.
//!
//! Provides erase and program operations on the internal program flash,
//! implementing the [`embedded_storage::nor_flash`] traits.
//!
//! # Safety
//!
//! The MK20 has a single program flash block. While a flash command is
//! executing (CCIF=0), ALL reads from program flash cause a read collision
//! error. The command launch and poll sequence therefore runs from RAM
//! via `#[link_section = ".data"]`, and interrupts are disabled during
//! the flash-unavailable window (ISR code lives in flash).
//!
//! # Protection
//!
//! Sector 0 (0x000-0x7FF) contains the vector table and flash
//! configuration field (0x400-0x40F). Erasing the config field without
//! restoring FSEC=0xFE **bricks the chip**. The driver refuses to
//! erase or write any address below [`SAFETY_FLOOR`].

use crate::pac;
use core::ptr;
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

// --- Hardware constants ---

/// FTFL peripheral base address.
const FTFL_BASE: u32 = 0x4002_0000;

/// FSTAT register address.
const FTFL_FSTAT_ADDR: *mut u8 = FTFL_BASE as *mut u8;

/// Sector size: 2 KB (smallest erase unit).
pub const SECTOR_SIZE: u32 = 2048;

/// First writable flash address. Everything below is off-limits.
/// Sector 0 (0x000-0x7FF) contains vector table + flash config field.
pub const SAFETY_FLOOR: u32 = 0x800;

/// Flash capacity in bytes.
#[cfg(feature = "mk20d7")]
const FLASH_SIZE: u32 = 256 * 1024;
#[cfg(feature = "mk20d5")]
const FLASH_SIZE: u32 = 128 * 1024;

/// Size of the region protected by each FPROT bit (32 bits total).
const PROTECTION_REGION_SIZE: u32 = FLASH_SIZE / 32;

// FTFL command codes
const CMD_ERASE_SECTOR: u8 = 0x09;
const CMD_PROGRAM_LONGWORD: u8 = 0x06;

// FSTAT bit masks
const FSTAT_CCIF: u8 = 0x80;
const FSTAT_ACCERR: u8 = 0x20;
const FSTAT_FPVIOL: u8 = 0x10;
const FSTAT_MGSTAT0: u8 = 0x01;

// FCCOB register offsets from FTFL_BASE.
// Kinetis FCCOB layout is big-endian within each 4-byte group:
//   [0x04] FCCOB3  [0x05] FCCOB2  [0x06] FCCOB1  [0x07] FCCOB0
//   [0x08] FCCOB7  [0x09] FCCOB6  [0x0A] FCCOB5  [0x0B] FCCOB4
const FCCOB0_OFFSET: u32 = 0x07; // Command code
const FCCOB1_OFFSET: u32 = 0x06; // Address[23:16]
const FCCOB2_OFFSET: u32 = 0x05; // Address[15:8]
const FCCOB3_OFFSET: u32 = 0x04; // Address[7:0]
const FCCOB4_OFFSET: u32 = 0x0B; // Data byte 0
const FCCOB5_OFFSET: u32 = 0x0A; // Data byte 1
const FCCOB6_OFFSET: u32 = 0x09; // Data byte 2
const FCCOB7_OFFSET: u32 = 0x08; // Data byte 3

// FPROT register base offset from FTFL_BASE.
// [0x10] FPROT3 (lowest flash)  [0x11] FPROT2  [0x12] FPROT1  [0x13] FPROT0 (highest flash)
const FPROT_OFFSET: u32 = 0x10;

// FSEC register offset from FTFL_BASE.
const FSEC_OFFSET: u32 = 0x02;

// --- Error type ---

/// Flash operation error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashError {
    /// Address or length not properly aligned.
    NotAligned,
    /// Address out of flash bounds.
    OutOfBounds,
    /// Attempted to modify protected region (FPROT or safety floor).
    Protected,
    /// FSTAT ACCERR — invalid command or parameters.
    AccessError,
    /// FSTAT FPVIOL — flash protection violation.
    ProtectionViolation,
    /// FSTAT MGSTAT0 — command execution failed verification.
    CommandFailure,
}

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            FlashError::NotAligned => NorFlashErrorKind::NotAligned,
            FlashError::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            _ => NorFlashErrorKind::Other,
        }
    }
}

// --- Driver struct ---

/// FTFL flash memory driver.
///
/// Created by calling [`FlashExt::flash`] on the PAC `Ftfl` peripheral.
/// All state lives in hardware registers — the struct is zero-sized.
/// Consuming the PAC `Ftfl` prevents aliased register access.
pub struct Flash {
    _ftfl: (),
}

/// Extension trait for the FTFL peripheral.
pub trait FlashExt {
    /// Consume the PAC FTFL peripheral and return the flash driver.
    fn flash(self) -> Flash;
}

impl FlashExt for pac::Ftfl {
    fn flash(self) -> Flash {
        Flash { _ftfl: () }
    }
}

// --- RAM trampoline ---

/// Clear error flags, launch a flash command, and poll until complete.
///
/// # Safety
///
/// Must be called with interrupts disabled. The caller must have
/// already written the FCCOB registers with command parameters.
/// Returns the FSTAT value after the command completes.
#[inline(never)]
#[link_section = ".data"]
unsafe fn launch_command(ftfl_fstat: *mut u8) -> u8 {
    // Clear ACCERR + FPVIOL (w1c)
    ptr::write_volatile(ftfl_fstat, 0x30);
    // Launch command by writing CCIF=1
    ptr::write_volatile(ftfl_fstat, 0x80);
    // Poll until CCIF=1
    loop {
        let fstat = ptr::read_volatile(ftfl_fstat);
        if fstat & 0x80 != 0 {
            return fstat;
        }
    }
}

impl Flash {
    /// Erase a single 2 KB sector containing `address`.
    ///
    /// The address does not need to be sector-aligned — it will be
    /// rounded down to the sector boundary. Returns an error if the
    /// address is out of bounds or in a protected region.
    pub fn erase_sector(&mut self, address: u32) -> Result<(), FlashError> {
        let aligned = address & !(SECTOR_SIZE - 1);

        if aligned >= FLASH_SIZE {
            return Err(FlashError::OutOfBounds);
        }
        if aligned < SAFETY_FLOOR {
            return Err(FlashError::Protected);
        }
        if self.is_protected(aligned) {
            return Err(FlashError::Protected);
        }

        cortex_m::interrupt::free(|_| unsafe {
            // Wait for any previous command to complete
            while ptr::read_volatile(FTFL_FSTAT_ADDR) & FSTAT_CCIF == 0 {}

            // Write command and address to FCCOB registers
            ptr::write_volatile((FTFL_BASE + FCCOB0_OFFSET) as *mut u8, CMD_ERASE_SECTOR);
            ptr::write_volatile(
                (FTFL_BASE + FCCOB1_OFFSET) as *mut u8,
                (aligned >> 16) as u8,
            );
            ptr::write_volatile(
                (FTFL_BASE + FCCOB2_OFFSET) as *mut u8,
                (aligned >> 8) as u8,
            );
            ptr::write_volatile((FTFL_BASE + FCCOB3_OFFSET) as *mut u8, aligned as u8);

            let fstat = launch_command(FTFL_FSTAT_ADDR);
            Self::check_errors(fstat)
        })
    }

    /// Program a longword (4 bytes) at the given address.
    ///
    /// `address` must be 4-byte aligned. The target longword must have
    /// been erased first — writing to already-programmed flash is
    /// undefined behavior.
    pub fn program_longword(&mut self, address: u32, data: &[u8; 4]) -> Result<(), FlashError> {
        if address & 0x3 != 0 {
            return Err(FlashError::NotAligned);
        }
        if address.saturating_add(4) > FLASH_SIZE {
            return Err(FlashError::OutOfBounds);
        }
        if address < SAFETY_FLOOR {
            return Err(FlashError::Protected);
        }
        if self.is_protected(address) {
            return Err(FlashError::Protected);
        }

        cortex_m::interrupt::free(|_| unsafe {
            while ptr::read_volatile(FTFL_FSTAT_ADDR) & FSTAT_CCIF == 0 {}

            // Command + address
            ptr::write_volatile(
                (FTFL_BASE + FCCOB0_OFFSET) as *mut u8,
                CMD_PROGRAM_LONGWORD,
            );
            ptr::write_volatile(
                (FTFL_BASE + FCCOB1_OFFSET) as *mut u8,
                (address >> 16) as u8,
            );
            ptr::write_volatile(
                (FTFL_BASE + FCCOB2_OFFSET) as *mut u8,
                (address >> 8) as u8,
            );
            ptr::write_volatile((FTFL_BASE + FCCOB3_OFFSET) as *mut u8, address as u8);

            // Data bytes: FCCOB4=byte0, FCCOB5=byte1, FCCOB6=byte2, FCCOB7=byte3
            ptr::write_volatile((FTFL_BASE + FCCOB4_OFFSET) as *mut u8, data[0]);
            ptr::write_volatile((FTFL_BASE + FCCOB5_OFFSET) as *mut u8, data[1]);
            ptr::write_volatile((FTFL_BASE + FCCOB6_OFFSET) as *mut u8, data[2]);
            ptr::write_volatile((FTFL_BASE + FCCOB7_OFFSET) as *mut u8, data[3]);

            let fstat = launch_command(FTFL_FSTAT_ADDR);
            Self::check_errors(fstat)
        })
    }

    /// Check if a flash address is in a hardware-protected region (FPROT registers).
    ///
    /// Returns `true` if the FPROT bit for the region containing `address`
    /// indicates protection (bit = 0 means protected).
    pub fn is_protected(&self, address: u32) -> bool {
        if address >= FLASH_SIZE {
            return true;
        }
        let region = address / PROTECTION_REGION_SIZE;
        // FPROT array in memory: [0x10]=FPROT3 (regions 0-7), [0x11]=FPROT2 (8-15),
        //                        [0x12]=FPROT1 (16-23), [0x13]=FPROT0 (24-31)
        let fprot_idx = (region / 8) as u32;
        let fprot_bit = (region % 8) as u8;
        let fprot_val =
            unsafe { ptr::read_volatile((FTFL_BASE + FPROT_OFFSET + fprot_idx) as *const u8) };
        // Bit = 0 means protected, bit = 1 means unprotected
        (fprot_val & (1 << fprot_bit)) == 0
    }

    /// Read the FSEC register (flash security status).
    ///
    /// Bits \[1:0\] (SEC field):
    /// - `0b10` = unsecured (normal operation)
    /// - Other values = secured (restricted debug/flash access)
    pub fn security_status(&self) -> u8 {
        unsafe { ptr::read_volatile((FTFL_BASE + FSEC_OFFSET) as *const u8) }
    }

    /// Check FSTAT for error flags and return the appropriate error.
    fn check_errors(fstat: u8) -> Result<(), FlashError> {
        if fstat & FSTAT_FPVIOL != 0 {
            Err(FlashError::ProtectionViolation)
        } else if fstat & FSTAT_ACCERR != 0 {
            Err(FlashError::AccessError)
        } else if fstat & FSTAT_MGSTAT0 != 0 {
            Err(FlashError::CommandFailure)
        } else {
            Ok(())
        }
    }
}

// --- embedded-storage trait implementations ---

impl ErrorType for Flash {
    type Error = FlashError;
}

impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 1;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        let end = offset
            .checked_add(bytes.len() as u32)
            .ok_or(FlashError::OutOfBounds)?;
        if end > FLASH_SIZE {
            return Err(FlashError::OutOfBounds);
        }
        // Flash is memory-mapped at 0x0000_0000 — direct read, no command needed.
        unsafe {
            ptr::copy_nonoverlapping(offset as *const u8, bytes.as_mut_ptr(), bytes.len());
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        FLASH_SIZE as usize
    }
}

impl NorFlash for Flash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = SECTOR_SIZE as usize;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        if from & (SECTOR_SIZE - 1) != 0 || to & (SECTOR_SIZE - 1) != 0 {
            return Err(FlashError::NotAligned);
        }
        if from > to || to > FLASH_SIZE {
            return Err(FlashError::OutOfBounds);
        }

        let mut addr = from;
        while addr < to {
            self.erase_sector(addr)?;
            addr += SECTOR_SIZE;
        }
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        if offset & 0x3 != 0 || bytes.len() & 0x3 != 0 {
            return Err(FlashError::NotAligned);
        }
        let end = offset
            .checked_add(bytes.len() as u32)
            .ok_or(FlashError::OutOfBounds)?;
        if end > FLASH_SIZE {
            return Err(FlashError::OutOfBounds);
        }

        for (i, chunk) in bytes.chunks_exact(4).enumerate() {
            let addr = offset + (i as u32) * 4;
            let data = [chunk[0], chunk[1], chunk[2], chunk[3]];
            self.program_longword(addr, &data)?;
        }
        Ok(())
    }
}
