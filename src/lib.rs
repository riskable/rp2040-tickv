//! # rp2040-tickv
//!
//! TicKV (Tiny Circular Key Value) is a small file system allowing
//! key value pairs to be stored in Flash Memory.  This is an implementation
//! of TicKV for RP2040 hardware that utilizes leftover space at the end
//! of flash memory for persistent key:value storage.
//!
//! You can learn more about TicKV here:
//! [TicKV](https://docs.tockos.org/tickv/index.html]).
//!
//! For this code to work properly (for now) you'll need to ensure that you have:
//!
//! `lto = 'fat'` or `lto = 'thin'`
//!
//! ...in your Cargo.toml under `[profile.release]` and similar sections (e.g. `dev-`).
//!
//! When implementing this you'll need to choose a hashing function for TicKV to use.
//! For this I recommend [SipHasher](https://crates.io/crates/siphasher).  See the
//! `basic_read_write.rs` example for how to use it with TicKV.
//!
//! # Limitations
//!
//! The longest key/value (combined) that can be stored is 4079 bytes. This is because
//! the RP2040 uses flash chips that have 4096 byte sectors (the minimum amount that
//! can be erased) and TicKV has a header overhead of 17 bytes.
//!
//! # Considerations
//!
//! You'll need to call `tickv.garbage_collect()` in your code from time to time after
//! you've marked keys as invalid or TicKV won't consider that space as "free".  A
//! simple way to handle this is to just collect garbage somewhere in your `init()`
//! function and/or after a certain number of keys have been marked invalid (depending
//! on how much storage you're using and how big the values are).
//!
//! Note that calling `tickv.garbage_collect()` is a relatively safe and low-overhead
//! operation if nothing has changed and only pauses interrupts briefly if it has to
//! perform erase (cleanup) operations.  So it's probably not a big deal if you run
//! it inside of a very long timer (say, once an hour).  The longer you go between
//! garbage collections the longer it could take though (it's one of those things).
//!
//! Ultimately though, it's up to you to figure out when to call it since it's highly
//! dependent on the application.  How long erasure takes is also highly dependent on
//! your hardware.  So do some testing and make some guesses 👍

#![no_std]
use core::slice;
use hal::rom_data;
use rp2040_hal as hal; // Shortcut
use tickv::{ErrorCode, FlashController};

pub const BLOCK_SIZE: u32 = 65536; // Larger than flash so block_cmd is ignored
pub const SECTOR_SIZE: usize = 4096; // 4k blocks are required by RP2040

/* IMPORTANT NOTE ABOUT RP2040 FLASH SPACE ADDRESSES:
When you pass an `addr` to a `rp2040-hal::rom_data` function it wants
addresses that start at `0x0000_0000`. However, when you want to read
that data back using something like `slice::from_raw_parts()` you
need the address space to start at `0x1000_0000` (aka `FLASH_XIP_BASE`).
*/
pub const FLASH_XIP_BASE: u32 = 0x1000_0000;

pub struct RP2040FlashCtrl {
    pub flash_end: u32,     // e.g. 0x0020_0000
    pub storage_size: u32,  // e.g. 128*4096 (has to be multiple of 4096)
    pub base_addr: u32,     // Calculated from flash_end - storage_size
    pub xip_base_addr: u32, // For doing reads
}

impl RP2040FlashCtrl {
    pub fn new(flash_end: u32, storage_size: u32) -> Result<Self, ErrorCode> {
        if storage_size % SECTOR_SIZE as u32 != 0 {
            // Must be multiple of 4096
            Err(ErrorCode::BufferTooSmall(SECTOR_SIZE))
        } else {
            let base_addr = flash_end - storage_size;
            let xip_base_addr = FLASH_XIP_BASE + flash_end - storage_size;
            Ok(RP2040FlashCtrl {
                flash_end,
                storage_size,
                base_addr,
                xip_base_addr,
            })
        }
    }
}

impl<'a> FlashController<SECTOR_SIZE> for RP2040FlashCtrl {
    fn read_region(
        // Reads don't need to be in RAM
        &self,
        region_number: usize,
        offset: usize,
        buf: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), ErrorCode> {
        let addr = (self.xip_base_addr + ((region_number * SECTOR_SIZE) as u32 + offset as u32))
            as *mut u8;
        let slice = unsafe { slice::from_raw_parts(addr, buf.len()) };
        buf.copy_from_slice(&slice);
        Ok(())
    }

    #[inline(never)]
    #[link_section = ".data.ram_func"]
    fn write(&self, address: usize, buf: &[u8]) -> Result<(), ErrorCode> {
        let addr = self.base_addr + address as u32;
        unsafe {
            cortex_m::interrupt::free(|_cs| {
                rom_data::connect_internal_flash();
                rom_data::flash_exit_xip();
                rom_data::flash_range_program(addr, buf.as_ptr(), buf.len());
                rom_data::flash_flush_cache(); // Get the XIP working again
                rom_data::flash_enter_cmd_xip(); // Start XIP back up
            });
        }
        Ok(())
    }

    #[inline(never)]
    #[link_section = ".data.ram_func"]
    fn erase_region(&self, region_number: usize) -> Result<(), ErrorCode> {
        let addr = self.base_addr + (region_number * SECTOR_SIZE) as u32;
        unsafe {
            cortex_m::interrupt::free(|_cs| {
                rom_data::connect_internal_flash();
                rom_data::flash_exit_xip();
                rom_data::flash_range_erase(addr, SECTOR_SIZE, BLOCK_SIZE, 0);
                rom_data::flash_flush_cache(); // Get the XIP working again
                rom_data::flash_enter_cmd_xip(); // Start XIP back up
            });
        }
        Ok(())
    }
}
