//! HAL interface to the UARTE peripheral.
//!
//! See product specification:
//!
//! - nrf52832: Section 35
//! - nrf52840: Section 6.34
use core::fmt;
use core::ops::Deref;
use core::sync::atomic::{compiler_fence, Ordering::SeqCst};

#[cfg(any(feature = "52833", feature = "52840"))]
use crate::pac::UARTE1;

#[cfg(feature = "9160")]
use crate::pac::{uarte0_ns as uarte0, UARTE0_NS as UARTE0, UARTE1_NS as UARTE1};

#[cfg(not(feature = "9160"))]
use crate::pac::{uarte0, UARTE0};

use crate::gpio::{AnyPin, PinExt};
use crate::prelude::*;
use crate::slice_in_ram_or;
use crate::target_constants::EASY_DMA_SIZE;
use crate::timer::{self, Timer};

// Re-export SVD variants to allow user to directly set values.
pub use uarte0::{baudrate::BAUDRATE_A as Baudrate, config::PARITY_A as Parity};

/// Interface to a UARTE instance.
///
/// This is a very basic interface that comes with the following limitations:
/// - The UARTE instances share the same address space with instances of UART.
///   You need to make sure that conflicting instances
///   are disabled before using `Uarte`. See product specification:
///     - nrf52832: Section 15.2
///     - nrf52840: Section 6.1.2
pub struct Uarte<T>(T);

impl<T> Uarte<T>
where
    T: Instance,
{
    pub fn new(uarte: T, pins: Pins, parity: Parity, baudrate: Baudrate) -> Self {
        // Select pins
        pins.rxd.conf().write(|w| w.input().connect());
        uarte.psel.rxd.write(|w| {
            unsafe { w.bits(pins.rxd.psel_bits()) };
            w.connect().connected()
        });

        pins.txd
            .block()
            .outset
            .write(|w| unsafe { w.bits(1 << pins.txd.pin()) });
        pins.txd.conf().write(|w| w.dir().output());
        uarte.psel.txd.write(|w| {
            unsafe { w.bits(pins.txd.psel_bits()) };
            w.connect().connected()
        });

        // Optional pins
        if let Some(ref pin) = pins.cts {
            pin.conf().write(|w| w.input().connect());
            uarte.psel.cts.write(|w| {
                unsafe { w.bits(pin.psel_bits()) };
                w.connect().connected()
            });
        } else {
            uarte.psel.cts.write(|w| w.connect().disconnected());
        }

        if let Some(ref pin) = pins.rts {
            pin.block()
                .outset
                .write(|w| unsafe { w.bits(1 << pin.pin()) });
            pin.conf().write(|w| w.dir().output());

            uarte.psel.rts.write(|w| {
                unsafe { w.bits(pin.psel_bits()) };
                w.connect().connected()
            });
        } else {
            uarte.psel.rts.write(|w| w.connect().disconnected());
        }

        // Enable UARTE instance.
        uarte.enable.write(|w| w.enable().enabled());

        // Configure.
        let hardware_flow_control = pins.rts.is_some() && pins.cts.is_some();
        uarte
            .config
            .write(|w| w.hwfc().bit(hardware_flow_control).parity().variant(parity));

        // Configure frequency.
        uarte.baudrate.write(|w| w.baudrate().variant(baudrate));

        Uarte(uarte)
    }

    /// Write via UARTE.
    ///
    /// This method uses transmits all bytes in `tx_buffer`.
    ///
    /// The buffer must have a length of at most 255 bytes on the nRF52832
    /// and at most 65535 bytes on the nRF52840.
    pub fn write(&mut self, tx_buffer: &[u8]) -> Result<(), Error> {
        if tx_buffer.len() > EASY_DMA_SIZE {
            return Err(Error::TxBufferTooLong);
        }

        // We can only DMA out of RAM.
        slice_in_ram_or(tx_buffer, Error::BufferNotInRAM)?;

        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // before any DMA action has started.
        compiler_fence(SeqCst);

        // Reset the events.
        self.0.events_endtx.reset();
        self.0.events_txstopped.reset();

        // Set up the DMA write.
        self.0.txd.ptr.write(|w|
            // We're giving the register a pointer to the stack. Since we're
            // waiting for the UARTE transaction to end before this stack pointer
            // becomes invalid, there's nothing wrong here.
            //
            // The PTR field is a full 32 bits wide and accepts the full range
            // of values.
            unsafe { w.ptr().bits(tx_buffer.as_ptr() as u32) });
        self.0.txd.maxcnt.write(|w|
            // We're giving it the length of the buffer, so no danger of
            // accessing invalid memory. We have verified that the length of the
            // buffer fits in an `u8`, so the cast to `u8` is also fine.
            //
            // The MAXCNT field is 8 bits wide and accepts the full range of
            // values.
            unsafe { w.maxcnt().bits(tx_buffer.len() as _) });

        // Start UARTE Transmit transaction.
        self.0.tasks_starttx.write(|w|
            // `1` is a valid value to write to task registers.
            unsafe { w.bits(1) });

        // Wait for transmission to end.
        let mut endtx;
        let mut txstopped;
        loop {
            endtx = self.0.events_endtx.read().bits() != 0;
            txstopped = self.0.events_txstopped.read().bits() != 0;
            if endtx || txstopped {
                break;
            }
        }

        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // after all possible DMA actions have completed.
        compiler_fence(SeqCst);

        if txstopped {
            return Err(Error::Transmit);
        }

        // Lower power consumption by disabling the transmitter once we're
        // finished.
        self.0.tasks_stoptx.write(|w|
            // `1` is a valid value to write to task registers.
            unsafe { w.bits(1) });

        Ok(())
    }

    /// Read via UARTE.
    ///
    /// This method fills all bytes in `rx_buffer`, and blocks
    /// until the buffer is full.
    ///
    /// The buffer must have a length of at most 255 bytes.
    pub fn read(&mut self, rx_buffer: &mut [u8]) -> Result<(), Error> {
        self.start_read(rx_buffer)?;

        // Wait for transmission to end.
        while self.0.events_endrx.read().bits() == 0 {}

        self.finalize_read();

        if self.0.rxd.amount.read().bits() != rx_buffer.len() as u32 {
            return Err(Error::Receive);
        }

        Ok(())
    }

    /// Read via UARTE.
    ///
    /// This method fills all bytes in `rx_buffer`, and blocks
    /// until the buffer is full or the timeout expires, whichever
    /// comes first.
    ///
    /// If the timeout occurs, an `Error::Timeout(n)` will be returned,
    /// where `n` is the number of bytes read successfully.
    ///
    /// This method assumes the interrupt for the given timer is NOT enabled,
    /// and in cases where a timeout does NOT occur, the timer will be left running
    /// until completion.
    ///
    /// The buffer must have a length of at most 255 bytes.
    pub fn read_timeout<I>(
        &mut self,
        rx_buffer: &mut [u8],
        timer: &mut Timer<I>,
        cycles: u32,
    ) -> Result<(), Error>
    where
        I: timer::Instance,
    {
        // Start the read.
        self.start_read(rx_buffer)?;

        // Start the timeout timer.
        timer.start(cycles);

        // Wait for transmission to end.
        let mut event_complete = false;
        let mut timeout_occured = false;

        loop {
            event_complete |= self.0.events_endrx.read().bits() != 0;
            timeout_occured |= timer.wait().is_ok();
            if event_complete || timeout_occured {
                break;
            }
        }

        if !event_complete {
            // Cancel the reception if it did not complete until now.
            self.cancel_read();
        }

        // Cleanup, even in the error case.
        self.finalize_read();

        let bytes_read = self.0.rxd.amount.read().bits() as usize;

        if timeout_occured && !event_complete {
            return Err(Error::Timeout(bytes_read));
        }

        if bytes_read != rx_buffer.len() as usize {
            return Err(Error::Receive);
        }

        Ok(())
    }

    /// Start a UARTE read transaction by setting the control
    /// values and triggering a read task.
    fn start_read(&mut self, rx_buffer: &mut [u8]) -> Result<(), Error> {
        if rx_buffer.len() > EASY_DMA_SIZE {
            return Err(Error::RxBufferTooLong);
        }

        // NOTE: RAM slice check is not necessary, as a mutable slice can only be
        // built from data located in RAM.

        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // before any DMA action has started.
        compiler_fence(SeqCst);

        // Set up the DMA read
        self.0.rxd.ptr.write(|w|
            // We're giving the register a pointer to the stack. Since we're
            // waiting for the UARTE transaction to end before this stack pointer
            // becomes invalid, there's nothing wrong here.
            //
            // The PTR field is a full 32 bits wide and accepts the full range
            // of values.
            unsafe { w.ptr().bits(rx_buffer.as_ptr() as u32) });
        self.0.rxd.maxcnt.write(|w|
            // We're giving it the length of the buffer, so no danger of
            // accessing invalid memory. We have verified that the length of the
            // buffer fits in an `u8`, so the cast to `u8` is also fine.
            //
            // The MAXCNT field is at least 8 bits wide and accepts the full
            // range of values.
            unsafe { w.maxcnt().bits(rx_buffer.len() as _) });

        // Start UARTE Receive transaction.
        self.0.tasks_startrx.write(|w|
            // `1` is a valid value to write to task registers.
            unsafe { w.bits(1) });

        Ok(())
    }

    /// Finalize a UARTE read transaction by clearing the event.
    fn finalize_read(&mut self) {
        // Reset the event, otherwise it will always read `1` from now on.
        self.0.events_endrx.write(|w| w);

        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // after all possible DMA actions have completed.
        compiler_fence(SeqCst);
    }

    /// Stop an unfinished UART read transaction and flush FIFO to DMA buffer.
    fn cancel_read(&mut self) {
        // Stop reception.
        self.0.tasks_stoprx.write(|w| unsafe { w.bits(1) });

        // Wait for the reception to have stopped.
        while self.0.events_rxto.read().bits() == 0 {}

        // Reset the event flag.
        self.0.events_rxto.write(|w| w);

        // Ask UART to flush FIFO to DMA buffer.
        self.0.tasks_flushrx.write(|w| unsafe { w.bits(1) });

        // Wait for the flush to complete.
        while self.0.events_endrx.read().bits() == 0 {}

        // The event flag itself is later reset by `finalize_read`.
    }

    /// Return the raw interface to the underlying UARTE peripheral.
    pub fn free(self) -> (T, Pins) {
        let rxd = self.0.psel.rxd.read();
        let txd = self.0.psel.txd.read();
        let cts = self.0.psel.cts.read();
        let rts = self.0.psel.rts.read();
        (
            self.0,
            Pins {
                rxd: unsafe { AnyPin::from_psel_bits(rxd.bits()) },
                txd: unsafe { AnyPin::from_psel_bits(txd.bits()) },
                cts: if cts.connect().bit_is_set() {
                    Some(unsafe { AnyPin::from_psel_bits(cts.bits()) })
                } else {
                    None
                },
                rts: if rts.connect().bit_is_set() {
                    Some(unsafe { AnyPin::from_psel_bits(rts.bits()) })
                } else {
                    None
                },
            },
        )
    }
}

impl<T> fmt::Write for Uarte<T>
where
    T: Instance,
{
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Copy all data into an on-stack buffer so we never try to EasyDMA from
        // flash.
        let buf = &mut [0; 16][..];
        for block in s.as_bytes().chunks(16) {
            buf[..block.len()].copy_from_slice(block);
            self.write(&buf[..block.len()]).map_err(|_| fmt::Error)?;
        }

        Ok(())
    }
}

pub struct Pins {
    pub rxd: AnyPin,
    pub txd: AnyPin,
    pub cts: Option<AnyPin>,
    pub rts: Option<AnyPin>,
}

#[derive(Debug)]
pub enum Error {
    TxBufferTooLong,
    RxBufferTooLong,
    Transmit,
    Receive,
    Timeout(usize),
    BufferNotInRAM,
}

pub trait Instance: Deref<Target = uarte0::RegisterBlock> + sealed::Sealed {}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for UARTE0 {}
impl Instance for UARTE0 {}

#[cfg(any(feature = "52833", feature = "52840", feature = "9160"))]
mod _uarte1 {
    use super::*;
    impl sealed::Sealed for UARTE1 {}
    impl Instance for UARTE1 {}
}
