#![no_main]
#![no_std]

#[allow(unused_imports)]
use panic_semihosting;

use cortex_m_semihosting::{debug, hprintln};
use rtic::app;

#[cfg(feature = "51")]
use nrf51_hal as hal;

#[cfg(feature = "52805")]
use nrf52805_hal as hal;

#[cfg(feature = "52810")]
use nrf52810_hal as hal;

#[cfg(feature = "52811")]
use nrf52811_hal as hal;

#[cfg(feature = "52832")]
use nrf52832_hal as hal;

#[cfg(feature = "52840")]
use nrf52840_hal as hal;

#[app(device = crate::hal::pac)]
const APP: () = {
    #[init]
    fn init(_: init::Context) {
        hprintln!("init").unwrap();
    }

    #[idle]
    fn idle(_: idle::Context) -> ! {
        hprintln!("idle").unwrap();

        debug::exit(debug::EXIT_SUCCESS);

        loop {
            continue;
        }
    }
};
