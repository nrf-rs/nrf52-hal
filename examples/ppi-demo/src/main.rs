#![no_std]
#![no_main]

#[cfg(not(any(
    feature = "51",
    feature = "52810",
    feature = "52811",
    feature = "52832",
    feature = "52833",
    feature = "52840"
)))]
compile_error!(
    "This example requires one of the following device features enabled:
        51
        52810
        52811
        52832
        52833
        52840"
);

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
        cell::RefCell,
        panic::PanicInfo,
        sync::atomic::{compiler_fence, Ordering},
    },
    cortex_m::interrupt::Mutex,
    cortex_m_rt::entry,
    hal::{
        pac::{interrupt, Interrupt, RADIO},
        ppi,
        prelude::*,
        timer::Timer,
        Clocks,
    },
    rtt_target::{rprintln, rtt_init_print},
};

static RADIO_REGS: Mutex<RefCell<Option<RADIO>>> = Mutex::new(RefCell::new(None));

#[entry]
fn main() -> ! {
    let p = hal::pac::Peripherals::take().unwrap();

    let _clocks = Clocks::new(p.CLOCK).enable_ext_hfosc();
    rtt_init_print!();

    let ppi_channels = ppi::Parts::new(p.PPI);
    let mut channel0 = ppi_channels.ppi0;

    channel0.set_task_endpoint(&p.RADIO.tasks_disable);
    channel0.set_event_endpoint(&p.TIMER0.events_compare[0]);
    channel0.enable();

    let radio = p.RADIO;
    radio.intenset.write(|w| w.disabled().set());
    cortex_m::interrupt::free(|cs| RADIO_REGS.borrow(cs).replace(Some(radio)));
    // NOTE(unsafe) There isn't any abstraction depending on this interrupt being masked
    unsafe {
        cortex_m::peripheral::NVIC::unmask(Interrupt::RADIO);
    }

    let mut timer = Timer::one_shot(p.TIMER0);
    timer.start(0xFFFFu32);

    loop {
        // Prevent empty loop optimizations
        compiler_fence(Ordering::SeqCst);
    }
}

#[interrupt]
fn RADIO() {
    cortex_m::interrupt::free(|cs| {
        if let Some(regs) = RADIO_REGS.borrow(cs).borrow_mut().as_mut() {
            if regs.events_disabled.read().bits() == 1 {
                rprintln!("We hit the RADIO disabled interrupt");

                // Clear the disabled flag
                // NOTE(unsafe) 0 is a valid value to write to this register
                regs.events_disabled.write(|w| unsafe { w.bits(0) });
            }
        }
    });
}

#[inline(never)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cortex_m::interrupt::disable();
    rprintln!("{}", info);
    loop {
        compiler_fence(Ordering::SeqCst);
    }
}
