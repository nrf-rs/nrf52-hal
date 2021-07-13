#![no_std]
#![no_main]

// Import the right HAL/PAC crate, depending on the target chip
#[cfg(feature = "51")]
pub use nrf51_hal as hal;
#[cfg(feature = "52805")]
pub use nrf52805_hal as hal;
#[cfg(feature = "52810")]
pub use nrf52810_hal as hal;
#[cfg(feature = "52811")]
pub use nrf52811_hal as hal;
#[cfg(feature = "52832")]
pub use nrf52832_hal as hal;
#[cfg(feature = "52833")]
pub use nrf52833_hal as hal;
#[cfg(feature = "52840")]
pub use nrf52840_hal as hal;

use {
    core::{
        panic::PanicInfo,
        sync::atomic::{compiler_fence, Ordering},
    },
    cortex_m_rt::entry,
    hal::{Clocks, Ecb},
    rtt_target::{rprint, rprintln, rtt_init_print},
};

const MSG: [u8; 16] = *b"Message to encry";
const KEY: [u8; 16] = *b"aaaaaaaaaaaaaaaa";
const CIPHER_MSG: [u8; 16] = [
    0xFE, 0xF1, 0x63, 0x82, 0xB4, 0x54, 0x6B, 0xE4, 0xEB, 0x9A, 0x5C, 0x0E, 0xB6, 0x0E, 0x49, 0x2F,
];

#[entry]
fn main() -> ! {
    let p = hal::pac::Peripherals::take().unwrap();

    let _clocks = Clocks::new(p.CLOCK).enable_ext_hfosc();
    rtt_init_print!();

    let mut ecb = Ecb::init(p.ECB);

    loop {
        rprintln!("Starting Encryption\n");
        rprintln!("Clear text: {}", core::str::from_utf8(&MSG[..]).unwrap());

        let cipher_text = ecb.encrypt_block(MSG, KEY).unwrap();
        rprint!("Cipher Text: ");
        for number in cipher_text.iter() {
            rprint!("{:x} ", *number);
        }
        assert_eq!(cipher_text, CIPHER_MSG);
        rprintln!("\r\n Encryption Done\n");

        cortex_m::asm::delay(136_000_000);
    }
}

#[inline(never)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    rprintln!("{}", info);
    loop {
        compiler_fence(Ordering::SeqCst);
    }
}
