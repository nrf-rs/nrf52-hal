#![no_std]
#![no_main]

use embedded_hal::digital::v2::OutputPin;
use {
    core::{
        panic::PanicInfo,
        sync::atomic::{compiler_fence, Ordering},
    },
    hal::{
        gpio::{Level, Output, Pin, PushPull},
        gpiote::Gpiote,
        lpcomp::*,
        pac::POWER,
    },
    nrf52840_hal as hal,
    rtt_target::{rprintln, rtt_init_print},
};

#[rtic::app(device = crate::hal::pac, peripherals = true)]
const APP: () = {
    struct Resources {
        gpiote: Gpiote,
        led1: Pin<Output<PushPull>>,
        lpcomp: LpComp,
        power: POWER,
    }

    #[init]
    fn init(ctx: init::Context) -> init::LateResources {
        let _clocks = hal::clocks::Clocks::new(ctx.device.CLOCK).enable_ext_hfosc();
        rtt_init_print!();

        let p0 = hal::gpio::p0::Parts::new(ctx.device.P0);
        let btn1 = p0.p0_11.into_pullup_input().degrade();
        let led1 = p0.p0_13.into_push_pull_output(Level::High).degrade();
        let in_pin = p0.p0_04.into_floating_input();
        let ref_pin = p0.p0_03.into_floating_input();

        let lpcomp = LpComp::new(ctx.device.LPCOMP, &in_pin);
        lpcomp
            .vref(VRef::ARef) // Set Vref to external analog reference
            .aref_pin(&ref_pin) // External analog reference pin
            .hysteresis(true)
            .analog_detect(Transition::Up) // Power up the device on upward transition
            .enable_interrupt(Transition::Cross) // Trigger `COMP_LPCOMP` interrupt on any transition
            .enable();

        let gpiote = Gpiote::new(ctx.device.GPIOTE);
        gpiote
            .channel0()
            .input_pin(&btn1)
            .hi_to_lo()
            .enable_interrupt();

        rprintln!("Power ON");

        // Check if the device was powered up by the comparator
        if ctx.device.POWER.resetreas.read().lpcomp().is_detected() {
            // Clear the lpcomp reset reason bit
            ctx.device
                .POWER
                .resetreas
                .modify(|_r, w| w.lpcomp().set_bit());
            rprintln!("Powered up by the comparator!");
        }

        rprintln!("Press button 1 to shut down");

        init::LateResources {
            gpiote,
            led1,
            lpcomp,
            power: ctx.device.POWER,
        }
    }

    #[idle]
    fn idle(_: idle::Context) -> ! {
        loop {
            cortex_m::asm::wfi();
        }
    }

    #[task(binds = GPIOTE, resources = [gpiote, power])]
    fn on_gpiote(ctx: on_gpiote::Context) {
        ctx.resources.gpiote.reset_events();
        rprintln!("Power OFF");
        ctx.resources
            .power
            .systemoff
            .write(|w| w.systemoff().enter());
    }

    #[task(binds = COMP_LPCOMP, resources = [lpcomp, led1])]
    fn on_comp(ctx: on_comp::Context) {
        ctx.resources.lpcomp.reset_events();
        match ctx.resources.lpcomp.read() {
            CompResult::Above => ctx.resources.led1.set_low().ok(),
            CompResult::Below => ctx.resources.led1.set_high().ok(),
        };
    }
};

#[inline(never)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cortex_m::interrupt::disable();
    rprintln!("{}", info);
    loop {
        compiler_fence(Ordering::SeqCst);
    }
}