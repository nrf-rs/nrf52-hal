/// Represents a digital input or output level.
#[derive(Debug, Eq, PartialEq)]
pub enum Level {
    Low,
    High,
}

/// Represents a pull setting for an input.
#[derive(Debug, Eq, PartialEq)]
pub enum Pull {
    None,
    Up,
    Down,
}

/// A GPIO port with up to 32 pins.
#[derive(Debug, Eq, PartialEq)]
pub enum Port {
    /// Port 0, available on all nRF52 and nRF51 MCUs.
    Port0,

    /// Port 1, only available on some nRF52 MCUs.
    #[cfg(any(feature = "52833", feature = "52840"))]
    Port1,
}

// TODO this trait should be sealed
pub trait Pin {
    fn pin_port(&self) -> u8;
}

pub struct AnyPin {
    pin_port: u8,
}

impl AnyPin {
    pub unsafe fn from_psel_bits(psel_bits: u32) -> Self {
        Self {
            pin_port: psel_bits as u8,
        }
    }
}

impl Pin for AnyPin {
    fn pin_port(&self) -> u8 {
        self.pin_port
    }
}

// TODO: If Pin is sealed, maybe these can be default methods in Pin instead.
// TODO: some of these shouldn't be public
pub trait PinExt {
    fn pin(&self) -> u8;
    fn port(&self) -> Port;
    fn psel_bits(&self) -> u32;

    // TODO: should these take &mut self and/or be unsafe?
    fn block(&self) -> &gpio::RegisterBlock;
    fn conf(&self) -> &gpio::PIN_CNF;

    /// Degrade to a generic pin struct, which can be used with peripherals
    fn degrade(self) -> AnyPin;
}

impl<T: Pin> PinExt for T {
    #[inline]
    fn pin(&self) -> u8 {
        #[cfg(any(feature = "52833", feature = "52840"))]
        {
            self.pin_port() & 0x1f
        }

        #[cfg(not(any(feature = "52833", feature = "52840")))]
        {
            self.pin_port()
        }
    }

    #[inline]
    fn port(&self) -> Port {
        #[cfg(any(feature = "52833", feature = "52840"))]
        {
            if self.pin_port() & 0x20 == 0 {
                Port::Port0
            } else {
                Port::Port1
            }
        }

        #[cfg(not(any(feature = "52833", feature = "52840")))]
        {
            Port::Port0
        }
    }

    #[inline]
    fn psel_bits(&self) -> u32 {
        self.pin_port() as u32
    }

    fn block(&self) -> &gpio::RegisterBlock {
        let ptr = match self.port() {
            Port::Port0 => P0::ptr(),
            #[cfg(any(feature = "52833", feature = "52840"))]
            Port::Port1 => P1::ptr(),
        };

        unsafe { &*ptr }
    }

    fn conf(&self) -> &gpio::PIN_CNF {
        &self.block().pin_cnf[self.pin() as usize]
    }

    fn degrade(self) -> AnyPin {
        AnyPin {
            pin_port: self.pin_port(),
        }
    }
}

pub struct Input<T: Pin> {
    pin: T,
}

impl<T: Pin> Input<T> {
    pub fn new(pin: T, pull: Pull) -> Self {
        pin.conf().write(|w| {
            w.dir().input();
            w.input().connect();
            match pull {
                Pull::None => {
                    w.pull().disabled();
                }
                Pull::Up => {
                    w.pull().pullup();
                }
                Pull::Down => {
                    w.pull().pulldown();
                }
            }
            w.drive().s0s1();
            w.sense().disabled();
            w
        });

        Self { pin }
    }
}

impl<T: Pin> Drop for Input<T> {
    fn drop(&mut self) {
        self.pin.conf().reset();
    }
}

impl<T: Pin> InputPin for Input<T> {
    type Error = Void;

    fn is_high(&self) -> Result<bool, Self::Error> {
        self.is_low().map(|v| !v)
    }

    fn is_low(&self) -> Result<bool, Self::Error> {
        Ok(self.pin.block().in_.read().bits() & (1 << self.pin.pin()) == 0)
    }
}

pub struct Output<T: Pin> {
    pin: T,
}

impl<T: Pin> Output<T> {
    // TODO opendrain
    pub fn new(pin: T, initial_output: Level) -> Self {
        pin.conf().write(|w| {
            w.dir().output();
            w.input().disconnect();
            w.pull().disabled();
            w.drive().s0s1();
            w.sense().disabled();
            w
        });

        Self { pin }
    }
}

impl<T: Pin> Drop for Output<T> {
    fn drop(&mut self) {
        self.pin.conf().reset();
    }
}

impl<T: Pin> OutputPin for Output<T> {
    type Error = Void;

    /// Set the output as high.
    fn set_high(&mut self) -> Result<(), Self::Error> {
        // NOTE(unsafe) atomic write to a stateless register - TODO(AJM) verify?
        // TODO - I wish I could do something like `.pins$i()`...
        unsafe {
            self.pin
                .block()
                .outset
                .write(|w| w.bits(1u32 << self.pin.pin()));
        }
        Ok(())
    }

    /// Set the output as low.
    fn set_low(&mut self) -> Result<(), Self::Error> {
        // NOTE(unsafe) atomic write to a stateless register - TODO(AJM) verify?
        // TODO - I wish I could do something like `.pins$i()`...
        unsafe {
            self.pin
                .block()
                .outclr
                .write(|w| w.bits(1u32 << self.pin.pin()));
        }
        Ok(())
    }
}

impl<T: Pin> StatefulOutputPin for Output<T> {
    /// Is the output pin set as high?
    fn is_set_high(&self) -> Result<bool, Self::Error> {
        self.is_set_low().map(|v| !v)
    }

    /// Is the output pin set as low?
    fn is_set_low(&self) -> Result<bool, Self::Error> {
        // NOTE(unsafe) atomic read with no side effects - TODO(AJM) verify?
        // TODO - I wish I could do something like `.pins$i()`...
        Ok(self.pin.block().out.read().bits() & (1 << self.pin.pin()) == 0)
    }
}

#[cfg(feature = "51")]
use crate::pac::{gpio, GPIO as P0};

#[cfg(feature = "9160")]
use crate::pac::{p0_ns as gpio, P0_NS as P0};

#[cfg(not(any(feature = "9160", feature = "51")))]
use crate::pac::{p0 as gpio, P0};

#[cfg(any(feature = "52833", feature = "52840"))]
use crate::pac::P1;

use crate::hal::digital::v2::{InputPin, OutputPin, StatefulOutputPin};
use void::Void;

macro_rules! gpio {
    (
        $PX:ident, $px:ident, $port_num:expr, [
            $($PXi:ident: ($pxi:ident, $pin_num:expr),)+
        ]
    ) => {
        /// GPIO
        pub mod $px {
            use super::{Pin, $PX};

            // ===============================================================
            // This chunk allows you to obtain an nrf-hal gpio from the
            // upstream nrf52 gpio definitions by defining a trait
            // ===============================================================
            /// GPIO parts
            pub struct Parts {
                $(
                    /// Pin
                    pub $pxi: $PXi,
                )+
            }

            impl Parts {
                pub fn new(_gpio: $PX) -> Self {
                    Self {
                        $(
                            $pxi: $PXi {
                                _private: (),
                            },
                        )+
                    }
                }
            }

            // ===============================================================
            // Implement each of the typed pins usable through the nrf-hal
            // defined interface
            // ===============================================================
            $(
                pub struct $PXi {
                    _private: (),
                }

                impl Pin for $PXi {
                    fn pin_port(&self) -> u8 {
                        $port_num * 32 + $pin_num
                    }
                }
            )+
        }
    }
}

// ===========================================================================
// Definition of all the items used by the macros above.
//
// For now, it is a little repetitive, especially as the nrf52 only has one
// 32-bit GPIO port (P0)
// ===========================================================================
gpio!(P0, p0, 0, [
    P0_00: (p0_00,  0),
    P0_01: (p0_01,  1),
    P0_02: (p0_02,  2),
    P0_03: (p0_03,  3),
    P0_04: (p0_04,  4),
    P0_05: (p0_05,  5),
    P0_06: (p0_06,  6),
    P0_07: (p0_07,  7),
    P0_08: (p0_08,  8),
    P0_09: (p0_09,  9),
    P0_10: (p0_10, 10),
    P0_11: (p0_11, 11),
    P0_12: (p0_12, 12),
    P0_13: (p0_13, 13),
    P0_14: (p0_14, 14),
    P0_15: (p0_15, 15),
    P0_16: (p0_16, 16),
    P0_17: (p0_17, 17),
    P0_18: (p0_18, 18),
    P0_19: (p0_19, 19),
    P0_20: (p0_20, 20),
    P0_21: (p0_21, 21),
    P0_22: (p0_22, 22),
    P0_23: (p0_23, 23),
    P0_24: (p0_24, 24),
    P0_25: (p0_25, 25),
    P0_26: (p0_26, 26),
    P0_27: (p0_27, 27),
    P0_28: (p0_28, 28),
    P0_29: (p0_29, 29),
    P0_30: (p0_30, 30),
    P0_31: (p0_31, 31),
]);

// The p1 types are present in the p0 module generated from the
// svd, but we want to export them in a p1 module from this crate.
#[cfg(any(feature = "52833", feature = "52840"))]
gpio!(P1, p1, 1, [
    P1_00: (p1_00,  0),
    P1_01: (p1_01,  1),
    P1_02: (p1_02,  2),
    P1_03: (p1_03,  3),
    P1_04: (p1_04,  4),
    P1_05: (p1_05,  5),
    P1_06: (p1_06,  6),
    P1_07: (p1_07,  7),
    P1_08: (p1_08,  8),
    P1_09: (p1_09,  9),
    P1_10: (p1_10, 10),
    P1_11: (p1_11, 11),
    P1_12: (p1_12, 12),
    P1_13: (p1_13, 13),
    P1_14: (p1_14, 14),
    P1_15: (p1_15, 15),
]);
