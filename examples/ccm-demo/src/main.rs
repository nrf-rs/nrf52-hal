#![no_std]
#![no_main]

// Import the right HAL/PAC crate, depending on the target chip
#[cfg(feature = "51")]
pub use nrf51_hal as hal;
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
    hal::{
        ccm::{CcmData, DataRate},
        rng::Rng,
        Ccm, Clocks,
    },
    rand_core::RngCore,
    rtt_target::{rprintln, rtt_init_print},
};

mod stopwatch;
use stopwatch::StopWatch;

const MSG: [u8; 251] = *b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. Duis justo
libero, commodo eget tincidunt quis, elementum at ipsum. Praesent pharetra imperdiet eros, at
vestibulum diam mattis ac. Nunc viverra cursus justo, sollicitudin placerat justo lectus.";

const KEY: [u8; 16] = *b"aaaaaaaaaaaaaaaa";
const HEADER_SIZE: usize = 3;
const MIC_SIZE: usize = 4;
const LENGTH_INDEX: usize = 1;

#[entry]
fn main() -> ! {
    let p = hal::pac::Peripherals::take().unwrap();

    let _clocks = Clocks::new(p.CLOCK).enable_ext_hfosc();
    rtt_init_print!();

    let mut rng = Rng::new(p.RNG);
    let mut iv = [0u8; 8];
    rng.fill_bytes(&mut iv);

    let mut ccm_data_enc = CcmData::new(KEY, iv);
    let mut ccm_data_dec = CcmData::new(KEY, iv);

    #[cfg(feature = "51")]
    let mut ccm = Ccm::init(p.CCM, p.AAR, DataRate::_1Mbit);
    #[cfg(not(feature = "51"))]
    let mut ccm = Ccm::init(p.CCM, p.AAR, DataRate::_2Mbit);

    let mut clear_buffer = [0u8; 254];
    let mut cipher_buffer = [0u8; 258];
    let mut strach_area = [0u8; 271];

    (&mut clear_buffer[HEADER_SIZE..]).copy_from_slice(&MSG[..]);

    let mut stop_watch = StopWatch::new(p.TIMER0);

    let payload_lengths: [usize; 5] = [251, 128, 64, 32, 16];

    for &length in payload_lengths.iter() {
        // Adjust payload length
        clear_buffer[LENGTH_INDEX] = length as u8;

        rprintln!("Starting Encryption of {} bytes", length);
        stop_watch.start();

        ccm.encrypt_packet(
            &mut ccm_data_enc,
            &clear_buffer[..],
            &mut cipher_buffer[..],
            &mut strach_area[..],
        )
        .unwrap();

        let now = stop_watch.now();
        stop_watch.stop();

        assert_eq!(cipher_buffer[LENGTH_INDEX], (length + MIC_SIZE) as u8);

        rprintln!("Encryption Took: {} us", now);

        // Clears the buffer, so we can inspect the decrypted text
        clear_buffer = [0u8; 254];

        rprintln!("\r\nStarting Decryption of {} bytes", length + MIC_SIZE);
        stop_watch.start();

        ccm.decrypt_packet(
            &mut ccm_data_dec,
            &mut clear_buffer[..],
            &cipher_buffer[..],
            &mut strach_area[..],
        )
        .unwrap();

        let now = stop_watch.now();
        stop_watch.stop();

        rprintln!("Decryption Took: {} us\n\n", now);

        assert_eq!(clear_buffer[LENGTH_INDEX], length as u8);
        assert_eq!(
            &clear_buffer[HEADER_SIZE..length + HEADER_SIZE],
            &MSG[..length]
        );

        // Clears the cipher text for next round
        cipher_buffer = [0u8; 258];
    }

    rprintln!("Done");

    loop {
        compiler_fence(Ordering::SeqCst);
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
