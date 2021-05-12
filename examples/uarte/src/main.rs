#![no_main]
#![no_std]

use nrf52840_hal as hal;
use rtt_target::{rprintln, rtt_init_print};

use hal::gpio::PinExt;
use hal::uarte::{self, Baudrate, Parity};
use hal::Uarte;

#[panic_handler] // panicking behavior
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}

#[cortex_m_rt::entry]
fn main() -> ! {
    rtt_init_print!();
    let p = hal::pac::Peripherals::take().unwrap();
    let port0 = hal::gpio::p0::Parts::new(p.P0);

    let pins = uarte::Pins {
        rxd: port0.p0_08.degrade(),
        txd: port0.p0_06.degrade(),
        rts: None,
        cts: None,
    };
    rprintln!("dgjaksdjf!");

    let mut u = Uarte::new(p.UARTE0, pins, Parity::EXCLUDED, Baudrate::BAUD115200);

    rprintln!("hai!");

    let mut salute = *b"Hello there!";
    u.write(&salute).unwrap();

    rprintln!("written!");

    loop {
        let mut rx_buf = [0u8; 1];
        u.read(&mut rx_buf).unwrap();
        rprintln!("readed!");
    }
}
