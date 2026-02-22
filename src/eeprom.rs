//! EEPROM / FlexMemory driver.
//!
//! The MK20DX FlexMemory system provides hardware-managed EEPROM emulation
//! using FlexNVM (32 KB at 0x1000_0000) as backing store and FlexRAM
//! (2 KB at 0x1400_0000) as the read/write interface. The EEE (Enhanced
//! EEPROM) state machine handles wear-leveling automatically.
//!
//! # Architecture
//!
//! - **Reads** are instant — FlexRAM is memory-mapped
//! - **Writes** trigger the EEE state machine which programs to FlexNVM
//!   in the background. Must wait for EEERDY before next write.
//! - The FTFL controller is shared with program flash (`flash.rs`)
//!
//! # Usage
//!
//! ```no_run
//! use mk20dx_hal::prelude::*;
//!
//! let dp = pac::Peripherals::take().unwrap();
//! let (flash, mut eeprom) = dp.ftfl.flash();
//!
//! if eeprom.is_eee_enabled() {
//!     eeprom.write(0, 0x42).unwrap();
//!     let val = eeprom.read(0);
//! }
//! ```

use core::ptr;

use crate::flash::{FTFL_FSTAT_ADDR, FSTAT_CCIF, FSTAT_ACCERR, FSTAT_FPVIOL, FSTAT_MGSTAT0, FTFL_BASE, FCCOB0_OFFSET, FCCOB1_OFFSET, FCCOB4_OFFSET, FCCOB5_OFFSET};

/// FlexRAM base address (memory-mapped EEPROM interface).
const FLEXRAM_BASE: u32 = 0x1400_0000;

/// Maximum FlexRAM size in bytes.
const FLEXRAM_SIZE: u16 = 2048;

/// FTFL command: Set FlexRAM Function.
const CMD_SET_FLEXRAM: u8 = 0x81;

/// FTFL command: Program Partition.
const CMD_PROGRAM_PARTITION: u8 = 0x80;

/// EEPROM operation error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EepromError {
    /// FlexNVM not partitioned for EEPROM (EEERDY not set).
    NotPartitioned,
    /// FlexRAM not ready for EEPROM operations.
    NotReady,
    /// Offset exceeds EEPROM capacity.
    OutOfBounds,
    /// FSTAT ACCERR — invalid command or parameters.
    AccessError,
    /// FSTAT FPVIOL — protection violation.
    ProtectionViolation,
    /// FSTAT MGSTAT0 — command execution failed.
    CommandFailure,
}

/// EEPROM data set size (EEESPLIT encoding for FCCOB4).
///
/// Controls how FlexNVM is partitioned for EEPROM backup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EepromSize {
    /// 2048 bytes of EEPROM.
    Bytes2048,
    /// 1024 bytes of EEPROM.
    Bytes1024,
    /// 512 bytes of EEPROM.
    Bytes512,
    /// 256 bytes of EEPROM.
    Bytes256,
    /// 128 bytes of EEPROM.
    Bytes128,
    /// 64 bytes of EEPROM.
    Bytes64,
    /// 32 bytes of EEPROM.
    Bytes32,
    /// No EEPROM (all FlexNVM used as data flash).
    None,
}

impl EepromSize {
    /// Encoding for FCCOB4 (EEPROM Data Set Size field).
    fn code(self) -> u8 {
        match self {
            EepromSize::Bytes2048 => 0x03,
            EepromSize::Bytes1024 => 0x04,
            EepromSize::Bytes512 => 0x05,
            EepromSize::Bytes256 => 0x06,
            EepromSize::Bytes128 => 0x07,
            EepromSize::Bytes64 => 0x08,
            EepromSize::Bytes32 => 0x09,
            EepromSize::None => 0x3F,
        }
    }

    /// Number of EEPROM bytes for this size.
    pub fn bytes(self) -> u16 {
        match self {
            EepromSize::Bytes2048 => 2048,
            EepromSize::Bytes1024 => 1024,
            EepromSize::Bytes512 => 512,
            EepromSize::Bytes256 => 256,
            EepromSize::Bytes128 => 128,
            EepromSize::Bytes64 => 64,
            EepromSize::Bytes32 => 32,
            EepromSize::None => 0,
        }
    }
}

/// FlexNVM partition code for EEPROM backup size (FCCOB5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexNvmPartition {
    /// All 32 KB for data flash, no EEPROM backup.
    DataFlash32K,
    /// 24 KB data flash, 8 KB EEPROM backup.
    DataFlash24K,
    /// No data flash, all 32 KB for EEPROM backup.
    EeBackup32K,
}

impl FlexNvmPartition {
    /// Encoding for FCCOB5 (FlexNVM Partition Code).
    fn code(self) -> u8 {
        match self {
            FlexNvmPartition::DataFlash32K => 0x00,
            FlexNvmPartition::DataFlash24K => 0x08,
            FlexNvmPartition::EeBackup32K => 0x03,
        }
    }
}

/// EEPROM / FlexMemory driver.
///
/// Created alongside [`Flash`] by calling [`FlashExt::flash`](crate::flash::FlashExt::flash)
/// on the PAC `Ftfl` peripheral.
pub struct Eeprom {
    _ftfl: (),
}

impl Eeprom {
    /// Create a new Eeprom driver (called internally by FlashExt).
    pub(crate) fn new() -> Self {
        Eeprom { _ftfl: () }
    }

    /// Check if the EEE (Enhanced EEPROM) subsystem is enabled and ready.
    ///
    /// Returns `true` if FlexRAM is configured for EEPROM mode and ready
    /// for read/write operations (FCNFG.EEERDY = 1).
    pub fn is_eee_enabled(&self) -> bool {
        unsafe {
            let fcnfg = ptr::read_volatile((FTFL_BASE + 0x01) as *const u8);
            fcnfg & 0x01 != 0 // EEERDY is bit 0
        }
    }

    /// Check if FlexRAM is in traditional RAM mode (FCNFG.RAMRDY = 1).
    pub fn is_ram_mode(&self) -> bool {
        unsafe {
            let fcnfg = ptr::read_volatile((FTFL_BASE + 0x01) as *const u8);
            fcnfg & 0x02 != 0 // RAMRDY is bit 1
        }
    }

    /// Read a single byte from EEPROM at the given offset.
    ///
    /// Reads are instant — FlexRAM is memory-mapped. No error checking
    /// is performed; ensure `is_eee_enabled()` returns `true` first.
    ///
    /// # Panics
    ///
    /// Panics if `offset >= FLEXRAM_SIZE`.
    pub fn read(&self, offset: u16) -> u8 {
        assert!((offset as u32) < FLEXRAM_SIZE as u32);
        unsafe { ptr::read_volatile((FLEXRAM_BASE + offset as u32) as *const u8) }
    }

    /// Read multiple bytes from EEPROM into `buf` starting at `offset`.
    ///
    /// Returns `Err(OutOfBounds)` if the read would exceed capacity.
    pub fn read_slice(&self, offset: u16, buf: &mut [u8]) -> Result<(), EepromError> {
        let end = (offset as u32).checked_add(buf.len() as u32)
            .ok_or(EepromError::OutOfBounds)?;
        if end > FLEXRAM_SIZE as u32 {
            return Err(EepromError::OutOfBounds);
        }
        unsafe {
            ptr::copy_nonoverlapping(
                (FLEXRAM_BASE + offset as u32) as *const u8,
                buf.as_mut_ptr(),
                buf.len(),
            );
        }
        Ok(())
    }

    /// Write a single byte to EEPROM at the given offset.
    ///
    /// The EEE state machine handles wear-leveling and FlexNVM programming
    /// automatically. This method polls EEERDY until the write completes.
    ///
    /// Returns `Err(NotPartitioned)` if EEPROM is not enabled.
    pub fn write(&mut self, offset: u16, value: u8) -> Result<(), EepromError> {
        if !self.is_eee_enabled() {
            return Err(EepromError::NotPartitioned);
        }
        if offset >= FLEXRAM_SIZE {
            return Err(EepromError::OutOfBounds);
        }

        // Write byte to FlexRAM — triggers EEE state machine
        unsafe {
            ptr::write_volatile((FLEXRAM_BASE + offset as u32) as *mut u8, value);
        }

        // Poll EEERDY until write completes
        self.wait_eeerdy()
    }

    /// Write multiple bytes to EEPROM starting at `offset`.
    ///
    /// Each byte write triggers the EEE state machine. This method waits
    /// for EEERDY between each byte.
    pub fn write_slice(&mut self, offset: u16, data: &[u8]) -> Result<(), EepromError> {
        if !self.is_eee_enabled() {
            return Err(EepromError::NotPartitioned);
        }
        let end = (offset as u32).checked_add(data.len() as u32)
            .ok_or(EepromError::OutOfBounds)?;
        if end > FLEXRAM_SIZE as u32 {
            return Err(EepromError::OutOfBounds);
        }

        for (i, &byte) in data.iter().enumerate() {
            let addr = FLEXRAM_BASE + offset as u32 + i as u32;
            unsafe {
                ptr::write_volatile(addr as *mut u8, byte);
            }
            self.wait_eeerdy()?;
        }

        Ok(())
    }

    /// Switch FlexRAM between EEPROM mode and traditional RAM mode.
    ///
    /// - `eeprom = true`: FlexRAM functions as EEPROM (writes trigger EEE)
    /// - `eeprom = false`: FlexRAM functions as traditional RAM
    ///
    /// Uses FTFL command 0x81 (Set FlexRAM Function).
    pub fn set_flexram_mode(&mut self, eeprom: bool) -> Result<(), EepromError> {
        let flexram_code: u8 = if eeprom { 0x00 } else { 0xFF };

        cortex_m::interrupt::free(|_| unsafe {
            // Wait for previous command
            while ptr::read_volatile(FTFL_FSTAT_ADDR) & FSTAT_CCIF == 0 {}

            // Command: Set FlexRAM Function
            ptr::write_volatile((FTFL_BASE + FCCOB0_OFFSET) as *mut u8, CMD_SET_FLEXRAM);
            // FlexRAM function control code
            ptr::write_volatile((FTFL_BASE + FCCOB1_OFFSET) as *mut u8, flexram_code);

            let fstat = crate::flash::launch_command(FTFL_FSTAT_ADDR);
            Self::check_errors(fstat)
        })
    }

    /// Configure FlexNVM partition for EEPROM.
    ///
    /// **This is a one-time operation** until mass erase. The partition
    /// determines how FlexNVM is split between data flash and EEPROM
    /// backup storage.
    ///
    /// # Safety
    ///
    /// This permanently modifies the FlexNVM partition table. The chip must
    /// be mass-erased to change the partition afterward. The command runs
    /// from RAM (reuses the flash.rs launch_command trampoline).
    pub unsafe fn partition(
        &mut self,
        eeprom_size: EepromSize,
        partition: FlexNvmPartition,
    ) -> Result<(), EepromError> {
        cortex_m::interrupt::free(|_| {
            // Wait for previous command
            while ptr::read_volatile(FTFL_FSTAT_ADDR) & FSTAT_CCIF == 0 {}

            // Command: Program Partition
            ptr::write_volatile((FTFL_BASE + FCCOB0_OFFSET) as *mut u8, CMD_PROGRAM_PARTITION);
            // CSEc key size (0 = no CSEc)
            ptr::write_volatile((FTFL_BASE + FCCOB1_OFFSET) as *mut u8, 0x00);
            // EEPROM data set size
            ptr::write_volatile((FTFL_BASE + FCCOB4_OFFSET) as *mut u8, eeprom_size.code());
            // FlexNVM partition code
            ptr::write_volatile((FTFL_BASE + FCCOB5_OFFSET) as *mut u8, partition.code());

            let fstat = crate::flash::launch_command(FTFL_FSTAT_ADDR);
            Self::check_errors(fstat)
        })
    }

    /// Returns the maximum EEPROM capacity (FlexRAM size).
    ///
    /// The actual usable size depends on the partition configuration.
    /// This returns the FlexRAM size (2048 bytes).
    pub fn capacity(&self) -> usize {
        FLEXRAM_SIZE as usize
    }

    /// Poll FCNFG.EEERDY until set.
    fn wait_eeerdy(&self) -> Result<(), EepromError> {
        loop {
            let fcnfg = unsafe { ptr::read_volatile((FTFL_BASE + 0x01) as *const u8) };
            if fcnfg & 0x01 != 0 {
                return Ok(());
            }
            // Check FSTAT for errors while waiting
            let fstat = unsafe { ptr::read_volatile(FTFL_FSTAT_ADDR) };
            if fstat & FSTAT_ACCERR != 0 {
                return Err(EepromError::AccessError);
            }
            if fstat & FSTAT_FPVIOL != 0 {
                return Err(EepromError::ProtectionViolation);
            }
        }
    }

    /// Check FSTAT for error flags.
    fn check_errors(fstat: u8) -> Result<(), EepromError> {
        if fstat & FSTAT_FPVIOL != 0 {
            Err(EepromError::ProtectionViolation)
        } else if fstat & FSTAT_ACCERR != 0 {
            Err(EepromError::AccessError)
        } else if fstat & FSTAT_MGSTAT0 != 0 {
            Err(EepromError::CommandFailure)
        } else {
            Ok(())
        }
    }
}
