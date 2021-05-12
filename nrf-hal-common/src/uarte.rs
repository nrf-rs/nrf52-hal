//! HAL interface to the UARTE peripheral.
//!
//! See product specification:
//!
//! - nrf52832: Section 35
//! - nrf52840: Section 6.34
use core::fmt;
use core::ops::Deref;
use core::sync::atomic::{compiler_fence, Ordering::SeqCst};

use embedded_hal::blocking::serial as bserial;
use embedded_hal::digital::v2::OutputPin;
use embedded_hal::serial;

#[cfg(any(feature = "52833", feature = "52840"))]
use crate::pac::UARTE1;

#[cfg(feature = "9160")]
use crate::pac::{uarte0_ns as uarte0, UARTE0_NS as UARTE0, UARTE1_NS as UARTE1};

#[cfg(not(feature = "9160"))]
use crate::pac::{uarte0, UARTE0};

use crate::gpio::{Floating, Input, Output, Pin, PushPull};
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
    pub fn new(uarte: T, mut pins: Pins, parity: Parity, baudrate: Baudrate) -> Self {
        // Select pins
        uarte.psel.rxd.write(|w| {
            unsafe { w.bits(pins.rxd.psel_bits()) };
            w.connect().connected()
        });
        pins.txd.set_high().unwrap();
        uarte.psel.txd.write(|w| {
            unsafe { w.bits(pins.txd.psel_bits()) };
            w.connect().connected()
        });

        // Optional pins
        uarte.psel.cts.write(|w| {
            if let Some(ref pin) = pins.cts {
                unsafe { w.bits(pin.psel_bits()) };
                w.connect().connected()
            } else {
                w.connect().disconnected()
            }
        });

        uarte.psel.rts.write(|w| {
            if let Some(ref pin) = pins.rts {
                unsafe { w.bits(pin.psel_bits()) };
                w.connect().connected()
            } else {
                w.connect().disconnected()
            }
        });

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
        if tx_buffer.len() == 0 {
            return Err(Error::TxBufferTooSmall);
        }

        if tx_buffer.len() > EASY_DMA_SIZE {
            return Err(Error::TxBufferTooLong);
        }

        // We can only DMA out of RAM.
        slice_in_ram_or(tx_buffer, Error::BufferNotInRAM)?;

        start_write(&*self.0, tx_buffer);

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

        stop_write(&*self.0);
        Ok(())
    }

    /// Read via UARTE.
    ///
    /// This method fills all bytes in `rx_buffer`, and blocks
    /// until the buffer is full.
    ///
    /// The buffer must have a length of at most 255 bytes.
    pub fn read(&mut self, rx_buffer: &mut [u8]) -> Result<(), Error> {
        start_read(&*self.0, rx_buffer)?;

        // Wait for transmission to end.
        while self.0.events_endrx.read().bits() == 0 {}

        finalize_read(&*self.0);

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
        start_read(&self.0, rx_buffer)?;

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
            cancel_read(&self.0);
        }

        // Cleanup, even in the error case.
        finalize_read(&self.0);

        let bytes_read = self.0.rxd.amount.read().bits() as usize;

        if timeout_occured && !event_complete {
            return Err(Error::Timeout(bytes_read));
        }

        if bytes_read != rx_buffer.len() as usize {
            return Err(Error::Receive);
        }

        Ok(())
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
                rxd: unsafe { Pin::from_psel_bits(rxd.bits()) },
                txd: unsafe { Pin::from_psel_bits(txd.bits()) },
                cts: if cts.connect().bit_is_set() {
                    Some(unsafe { Pin::from_psel_bits(cts.bits()) })
                } else {
                    None
                },
                rts: if rts.connect().bit_is_set() {
                    Some(unsafe { Pin::from_psel_bits(rts.bits()) })
                } else {
                    None
                },
            },
        )
    }

    /// Split into implementations of embedded_hal::serial traits. The size of the slices passed to this
    /// method will determine the size of the DMA transfers performed.
    pub fn split(
        self,
        tx_buf: &'static mut [u8],
        rx_buf: &'static mut [u8; 1],
    ) -> Result<(UarteTx<T>, UarteRx<T>), Error> {
        let tx = UarteTx::new(tx_buf)?;
        let rx = UarteRx::new(rx_buf)?;
        Ok((tx, rx))
    }
}

/// Write via UARTE.
///
/// This method uses transmits all bytes in `tx_buffer`.
fn start_write(uarte: &uarte0::RegisterBlock, tx_buffer: &[u8]) {
    // Conservative compiler fence to prevent optimizations that do not
    // take in to account actions by DMA. The fence has been placed here,
    // before any DMA action has started.
    compiler_fence(SeqCst);

    // Reset the events.
    uarte.events_endtx.reset();
    uarte.events_txstopped.reset();

    // Set up the DMA write.
    uarte.txd.ptr.write(|w|
        // We're giving the register a pointer to the stack. Since we're
        // waiting for the UARTE transaction to end before this stack pointer
        // becomes invalid, there's nothing wrong here.
        //
        // The PTR field is a full 32 bits wide and accepts the full range
        // of values.
        unsafe { w.ptr().bits(tx_buffer.as_ptr() as u32) });
    uarte.txd.maxcnt.write(|w|
        // We're giving it the length of the buffer, so no danger of
        // accessing invalid memory. We have verified that the length of the
        // buffer fits in an `u8`, so the cast to `u8` is also fine.
        //
        // The MAXCNT field is 8 bits wide and accepts the full range of
        // values.
        unsafe { w.maxcnt().bits(tx_buffer.len() as _) });

    // Start UARTE Transmit transaction.
    uarte.tasks_starttx.write(|w|
        // `1` is a valid value to write to task registers.
        unsafe { w.bits(1) });
}

fn stop_write(uarte: &uarte0::RegisterBlock) {
    // `1` is a valid value to write to task registers.
    uarte.tasks_stoptx.write(|w| unsafe { w.bits(1) });

    // Wait for transmitter is stopped.
    while uarte.events_txstopped.read().bits() == 0 {}
}

/// Start a UARTE read transaction by setting the control
/// values and triggering a read task.
fn start_read(uarte: &uarte0::RegisterBlock, rx_buffer: &mut [u8]) -> Result<(), Error> {
    if rx_buffer.len() == 0 {
        return Err(Error::RxBufferTooSmall);
    }

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
    uarte.rxd.ptr.write(|w|
        // We're giving the register a pointer to the stack. Since we're
        // waiting for the UARTE transaction to end before this stack pointer
        // becomes invalid, there's nothing wrong here.
        //
        // The PTR field is a full 32 bits wide and accepts the full range
        // of values.
        unsafe { w.ptr().bits(rx_buffer.as_ptr() as u32) });
    uarte.rxd.maxcnt.write(|w|
        // We're giving it the length of the buffer, so no danger of
        // accessing invalid memory. We have verified that the length of the
        // buffer fits in an `u8`, so the cast to `u8` is also fine.
        //
        // The MAXCNT field is at least 8 bits wide and accepts the full
        // range of values.
        unsafe { w.maxcnt().bits(rx_buffer.len() as _) });

    // Start UARTE Receive transaction.
    uarte.tasks_startrx.write(|w|
            // `1` is a valid value to write to task registers.
            unsafe { w.bits(1) });

    Ok(())
}

/// Stop an unfinished UART read transaction and flush FIFO to DMA buffer.
fn cancel_read(uarte: &uarte0::RegisterBlock) {
    // Stop reception.
    uarte.tasks_stoprx.write(|w| unsafe { w.bits(1) });

    // Wait for the reception to have stopped.
    while uarte.events_rxto.read().bits() == 0 {}

    // Reset the event flag.
    uarte.events_rxto.write(|w| w);

    // Ask UART to flush FIFO to DMA buffer.
    uarte.tasks_flushrx.write(|w| unsafe { w.bits(1) });

    // Wait for the flush to complete.
    while uarte.events_endrx.read().bits() == 0 {}

    // The event flag itself is later reset by `finalize_read`.
}

/// Finalize a UARTE read transaction by clearing the event.
fn finalize_read(uarte: &uarte0::RegisterBlock) {
    // Reset the event, otherwise it will always read `1` from now on.
    uarte.events_endrx.write(|w| w);

    // Conservative compiler fence to prevent optimizations that do not
    // take in to account actions by DMA. The fence has been placed here,
    // after all possible DMA actions have completed.
    compiler_fence(SeqCst);
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
    pub rxd: Pin<Input<Floating>>,
    pub txd: Pin<Output<PushPull>>,
    pub cts: Option<Pin<Input<Floating>>>,
    pub rts: Option<Pin<Output<PushPull>>>,
}

#[derive(Debug)]
pub enum Error {
    TxBufferTooSmall,
    RxBufferTooSmall,
    TxBufferTooLong,
    RxBufferTooLong,
    Transmit,
    Receive,
    Timeout(usize),
    BufferNotInRAM,
}

pub trait Instance: Deref<Target = uarte0::RegisterBlock> + sealed::Sealed {
    fn ptr() -> *const uarte0::RegisterBlock;
}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for UARTE0 {}
impl Instance for UARTE0 {
    fn ptr() -> *const uarte0::RegisterBlock {
        UARTE0::ptr()
    }
}

#[cfg(any(feature = "52833", feature = "52840", feature = "9160"))]
mod _uarte1 {
    use super::*;
    impl sealed::Sealed for UARTE1 {}
    impl Instance for UARTE1 {
        fn ptr() -> *const uarte0::RegisterBlock {
            UARTE1::ptr()
        }
    }
}

/// Interface for the TX part of a UART instance that can be used independently of the RX part.
pub struct UarteTx<T>
where
    T: Instance,
{
    _marker: core::marker::PhantomData<T>,
    tx_buf: &'static mut [u8],
    written: usize,
}

/// Interface for the RX part of a UART instance that can be used independently of the TX part.
pub struct UarteRx<T>
where
    T: Instance,
{
    _marker: core::marker::PhantomData<T>,
    rx_buf: &'static mut [u8],
}

impl<T> UarteTx<T>
where
    T: Instance,
{
    fn new(tx_buf: &'static mut [u8]) -> Result<UarteTx<T>, Error> {
        if tx_buf.len() == 0 {
            return Err(Error::TxBufferTooSmall);
        }

        if tx_buf.len() > EASY_DMA_SIZE {
            return Err(Error::TxBufferTooLong);
        }

        Ok(UarteTx {
            _marker: core::marker::PhantomData,
            tx_buf,
            written: 0,
        })
    }
}

impl<T> UarteRx<T>
where
    T: Instance,
{
    fn new(rx_buf: &'static mut [u8]) -> Result<UarteRx<T>, Error> {
        if rx_buf.len() == 0 {
            return Err(Error::RxBufferTooSmall);
        }

        if rx_buf.len() > EASY_DMA_SIZE {
            return Err(Error::RxBufferTooLong);
        }

        Ok(UarteRx {
            _marker: core::marker::PhantomData,
            rx_buf,
        })
    }
}

impl<T> Drop for UarteTx<T>
where
    T: Instance,
{
    fn drop(&mut self) {
        let uarte = unsafe { &*T::ptr() };

        let in_progress = uarte.events_txstarted.read().bits() == 1;
        // Stop any ongoing transmission
        if in_progress {
            stop_write(uarte);

            // Reset events
            uarte.events_endtx.reset();
            uarte.events_txstopped.reset();

            // Ensure the above is done
            compiler_fence(SeqCst);
        }
    }
}

impl<T> Drop for UarteRx<T>
where
    T: Instance,
{
    fn drop(&mut self) {
        let uarte = unsafe { &*T::ptr() };

        let in_progress = uarte.events_rxstarted.read().bits() == 1;
        // Stop any ongoing reception
        if in_progress {
            cancel_read(uarte);

            // Reset events
            uarte.events_endrx.reset();

            // Ensure the above is done
            compiler_fence(SeqCst);
        }
    }
}

impl<T> serial::Write<u8> for UarteTx<T>
where
    T: Instance,
{
    type Error = Error;

    /// Write a single byte to the internal buffer. Returns nb::Error::WouldBlock if buffer is full.
    fn write(&mut self, b: u8) -> nb::Result<(), Self::Error> {
        let uarte = unsafe { &*T::ptr() };

        // Prevent writing to buffer while DMA transfer is in progress.
        if uarte.events_txstarted.read().bits() == 1 {
            return Err(nb::Error::WouldBlock);
        }

        if self.written < self.tx_buf.len() {
            self.tx_buf[self.written] = b;
            self.written += 1;
            Ok(())
        } else {
            self.flush()
        }
    }

    /// Flush the TX buffer non-blocking. Returns nb::Error::WouldBlock if not yet flushed.
    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        let uarte = unsafe { &*T::ptr() };

        // If txstarted is set, we are in the process of transmitting.
        let in_progress = uarte.events_txstarted.read().bits() == 1;

        if in_progress {
            let endtx = uarte.events_endtx.read().bits() != 0;
            let txstopped = uarte.events_txstopped.read().bits() != 0;
            if endtx || txstopped {
                // We are done, cleanup the state.
                uarte.events_txstarted.reset();
                self.written = 0;

                // Conservative compiler fence to prevent optimizations that do not
                // take in to account actions by DMA. The fence has been placed here,
                // after all possible DMA actions have completed.
                compiler_fence(SeqCst);

                if txstopped {
                    return Err(nb::Error::Other(Error::Transmit));
                }

                // Lower power consumption by disabling the transmitter once we're
                // finished.
                stop_write(uarte);

                compiler_fence(SeqCst);
                Ok(())
            } else {
                // Still not done, don't block.
                Err(nb::Error::WouldBlock)
            }
        } else {
            // No need to trigger transmit if we don't have anything written
            if self.written == 0 {
                return Ok(());
            }

            start_write(uarte, &self.tx_buf[0..self.written]);

            Err(nb::Error::WouldBlock)
        }
    }
}

// Auto-implement the blocking variant
impl<T> bserial::write::Default<u8> for UarteTx<T> where T: Instance {}

impl<T> core::fmt::Write for UarteTx<T>
where
    T: Instance,
{
    fn write_str(&mut self, s: &str) -> fmt::Result {
        s.as_bytes()
            .iter()
            .try_for_each(|c| nb::block!(self.write(*c)))
            .map_err(|_| core::fmt::Error)
    }
}

impl<T> serial::Read<u8> for UarteRx<T>
where
    T: Instance,
{
    type Error = Error;
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        let uarte = unsafe { &*T::ptr() };

        compiler_fence(SeqCst);

        let in_progress = uarte.events_rxstarted.read().bits() == 1;
        if in_progress && uarte.events_endrx.read().bits() == 0 {
            return Err(nb::Error::WouldBlock);
        }

        if in_progress {
            uarte.events_rxstarted.reset();

            finalize_read(uarte);

            if uarte.rxd.amount.read().bits() != 1 as u32 {
                return Err(nb::Error::Other(Error::Receive));
            }
            Ok(self.rx_buf[0])
        } else {
            // We can only read 1 byte at a time, otherwise ENDTX might not be raised,
            // causing the read to stall forever.
            start_read(&uarte, self.rx_buf)?;
            Err(nb::Error::WouldBlock)
        }
    }
}
