//! Sets up a TicKV keystore using 64KiB at the end of flash storage space.
//!
//! This will read and write a series of TicKV keys, "test1" through "test4" and output
//! the results via `defmt` to the terminal.  When running this example more than once
//! it will automatically handle the situation where keys already exist, mark them as
//! invald, and re-store the same key/values.
//!
//! This code expects that you're already setup to use `probe-run`.  To run this example:
//!
//! ```
//! cargo run --release --example basic_read_write
//! ```
//!
//! NOTE: You may need to modify this code to use a differnent rp2040_boot2
//! loader (e.g. BOOT_LOADER_AT25SF128A) to match your RP2040 hardware.
//!
#![no_std]
#![no_main]

use core::hash::{Hash, Hasher};
use cortex_m::delay::Delay;
use cortex_m_rt::entry;
use defmt::*;
use defmt_rtt as _;
use hal::{
    clocks::{init_clocks_and_plls, Clock},
    pac, rom_data,
    watchdog::Watchdog,
};
use panic_probe as _;
use rp2040_hal as hal;
use siphasher::sip::SipHasher;
use tickv::{ErrorCode, TicKV, MAIN_KEY};

// The important one:
use rp2040_tickv;

// How big is your flash? Default for this example is 2MiB
pub const FLASH_SIZE_MBYTES: u32 = 1;
// How much space to use for this test?
// SECTOR_SIZE is 4096 so 16*4096 is 64KiB:
pub const STORAGE_SIZE: u32 = 16 * rp2040_tickv::SECTOR_SIZE as u32;

pub const FLASH_END_ADDR: u32 = FLASH_SIZE_MBYTES * 1024 * 1024;
pub const STORAGE_ADDR: u32 = FLASH_END_ADDR - STORAGE_SIZE;

// These are here mostly for reference; they're 'block_cmd' you can pass to
// flash_range_erase() (as the 4th arg) in order to speed up erasure operations.
// They're specific to the brand/type of flash chip used so we don't use them
// by default...
pub const PAGE_ERASE: u8 = 0x02;
pub const SECTOR_ERASE: u8 = 0x20;
pub const BLOCK32_ERASE: u8 = 0x52;
pub const BLOCK64_ERASE: u8 = 0xD8;

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

#[allow(dead_code)]
#[inline(never)]
#[link_section = ".data.ram_func"] // Functions that modify flash always need to run from RAM
fn erase_flash_storage() {
    // Start with a fresh flash space
    info!("Erasing flash storage space {:?}", STORAGE_ADDR as *mut u8);
    unsafe {
        cortex_m::interrupt::free(|_cs| {
            rom_data::connect_internal_flash();
            rom_data::flash_exit_xip();
            rom_data::flash_range_erase(
                STORAGE_ADDR,
                STORAGE_SIZE as usize,
                rp2040_tickv::BLOCK_SIZE,
                BLOCK64_ERASE,
            );
            rom_data::flash_flush_cache(); // Get the XIP working again
            rom_data::flash_enter_cmd_xip(); // Start XIP back up
        });
    }
    info!("Flash storage erasure complete");
}

#[entry]
fn main() -> ! {
    info!("Program start");
    info!("Storage size: {:?} bytes", STORAGE_SIZE);
    info!("Flash end address: 0x10{:?}", FLASH_END_ADDR as *mut u8);
    let mut pac = pac::Peripherals::take().unwrap();
    let core = pac::CorePeripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);

    // External high-speed crystal on the pico board is 12Mhz
    let external_xtal_freq_hz = 12_000_000u32;
    let clocks = init_clocks_and_plls(
        external_xtal_freq_hz,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut delay = Delay::new(core.SYST, clocks.system_clock.freq().raw());
    let key_name1 = b"test1";
    let key_name2 = b"test2";
    let key_name3 = b"test3";
    let key_name4 = b"test4";
    let value1: [u8; 8] = [1; 8];
    let value2: [u8; 8] = [2; 8];
    let value3: [u8; 8] = [3; 8];
    let value4: [u8; 8] = [4; 8];

    // Uncomment these two lines if you want to start fresh with each run:
    // info!("Erasing TicKV flash space to start anew...");
    // erase_flash_storage();

    // Setup our TicKV stuff
    let controller = rp2040_tickv::RP2040FlashCtrl::new(FLASH_END_ADDR, STORAGE_SIZE).unwrap();
    let mut storage_buffer = &mut [0; rp2040_tickv::SECTOR_SIZE];
    let tickv = TicKV::<rp2040_tickv::RP2040FlashCtrl, { rp2040_tickv::SECTOR_SIZE }>::new(
        controller,
        &mut storage_buffer,
        STORAGE_SIZE as usize,
    );

    // info!("Setting up hasher");
    let mut hasher = SipHasher::new();
    MAIN_KEY.hash(&mut hasher);
    info!("Initializing TicKV storage...");
    let _ = tickv.initialise(hasher.finish()).unwrap();
    // Collect the garbage in case of subsequent calls to ensure there's always
    // room to store our four test keys/values:
    tickv.garbage_collect().unwrap();
    info!("TicKV storage ready!");

    // Add our keys and handle KeyAlreadyExists errors gracefully in the event of repeat runs
    let mut looping = true;
    while looping {
        info!("Writing new TicKV entry: 'test1'");
        match tickv.append_key(get_hashed_key(key_name1), &value1) {
            Ok(_) => {
                looping = false;
            }
            Err(e) => match e {
                ErrorCode::KeyAlreadyExists => {
                    info!("Key already exists.  Marking it as invalid...");
                    let _ = tickv.invalidate_key(get_hashed_key(key_name1)).unwrap();
                    // NOTE: We only do this invalid check once to save some space:
                    info!("Making sure it isn't there anymore...");
                    let mut buf: [u8; 8] = [0; 8];
                    match tickv.get_key(get_hashed_key(key_name1), &mut buf) {
                        Err(ErrorCode::KeyNotFound) => {
                            info!("Our key wasn't found (good!)");
                        }
                        Ok(_) => {
                            crate::panic!("Marking our key as invalid didn't work!");
                        }
                        Err(_) => {
                            crate::panic!("Got some other error check if the key was not there.");
                        }
                    }
                    continue;
                }
                ErrorCode::CorruptData => {
                    // info!("CorruptData!");
                    // erase_flash_storage();
                    crate::panic!("For whatever reason TicKV returned a CorruptData error!");
                }
                _ => {
                    crate::panic!("Got some other error trying to add the key.");
                }
            },
        }
        info!("Reading the TicKV entry ('test1') we just wrote...");
        let mut buf: [u8; 8] = [0; 8];
        tickv.get_key(get_hashed_key(key_name1), &mut buf).unwrap();
        info!("'test1' read successfully!  Here's its value: {:?}", buf);
    }
    delay.delay_ms(500);
    looping = true;
    while looping {
        info!("Writing new TicKV entry: 'test2'");
        match tickv.append_key(get_hashed_key(key_name2), &value2) {
            Ok(_) => {
                looping = false;
            }
            Err(e) => match e {
                ErrorCode::KeyAlreadyExists => {
                    info!("Key already exists.  Marking it as invalid...");
                    let _ = tickv.invalidate_key(get_hashed_key(key_name2)).unwrap();
                    // tickv.garbage_collect();
                    // info!("Done with garbage collection");
                    continue;
                }
                ErrorCode::CorruptData => {
                    // info!("CorruptData!");
                    // erase_flash_storage();
                    crate::panic!("For whatever reason TicKV returned a CorruptData error!");
                }
                _ => {
                    crate::panic!("Got some other error trying to add the key.");
                }
            },
        }
        info!("Reading the TicKV entry ('test2') we just wrote...");
        let mut buf: [u8; 8] = [0; 8];
        tickv.get_key(get_hashed_key(key_name2), &mut buf).unwrap();
        info!("'test2' read successfully!  Here's its value: {:?}", buf);
    }
    delay.delay_ms(500);
    looping = true;
    while looping {
        info!("Writing new TicKV entry: 'test3'");
        match tickv.append_key(get_hashed_key(key_name3), &value3) {
            Ok(_) => {
                looping = false;
            }
            Err(e) => match e {
                ErrorCode::KeyAlreadyExists => {
                    info!("Key already exists.  Marking it as invalid...");
                    let _ = tickv.invalidate_key(get_hashed_key(key_name3)).unwrap();
                    // tickv.garbage_collect();
                    // info!("Done with garbage collection");
                    continue;
                }
                ErrorCode::CorruptData => {
                    // info!("CorruptData!");
                    // erase_flash_storage();
                    crate::panic!("For whatever reason TicKV returned a CorruptData error!");
                }
                _ => {
                    crate::panic!("Got some other error trying to add the key.");
                }
            },
        }
        info!("Reading the TicKV entry ('test3') we just wrote...");
        let mut buf: [u8; 8] = [0; 8];
        tickv.get_key(get_hashed_key(key_name3), &mut buf).unwrap();
        info!("'test3' read successfully!  Here's its value: {:?}", buf);
    }
    delay.delay_ms(500);
    looping = true;
    while looping {
        info!("Writing new TicKV entry: 'test4'");
        match tickv.append_key(get_hashed_key(key_name4), &value4) {
            Ok(_) => {
                looping = false;
            }
            Err(e) => match e {
                ErrorCode::KeyAlreadyExists => {
                    info!("Key already exists.  Marking it as invalid...");
                    let _ = tickv.invalidate_key(get_hashed_key(key_name4)).unwrap();
                    // tickv.garbage_collect();
                    // info!("Done with garbage collection");
                    continue;
                }
                ErrorCode::CorruptData => {
                    // info!("CorruptData!");
                    // erase_flash_storage();
                    crate::panic!("For whatever reason TicKV returned a CorruptData error!");
                }
                _ => {
                    crate::panic!("Got some other error trying to add the key.");
                }
            },
        }
        info!("Reading the TicKV entry ('test4') we just wrote...");
        let mut buf: [u8; 8] = [0; 8];
        tickv.get_key(get_hashed_key(key_name4), &mut buf).unwrap();
        info!("'test4' read successfully!  Here's its value: {:?}", buf);
    }
    delay.delay_ms(100);
    info!("All TicKV actions complete!");

    loop {
        cortex_m::asm::nop(); // Just hang around until the user kills it
    }
}

fn get_hashed_key(unhashed_key: &[u8]) -> u64 {
    let mut hash_function = SipHasher::new();
    unhashed_key.hash(&mut hash_function);
    hash_function.finish()
}
