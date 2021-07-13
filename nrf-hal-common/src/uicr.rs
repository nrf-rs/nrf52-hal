//! HAL interface to the UICR core component.
//!
//! See product specification:
//!
//! - nrf52805: Section 4.5
//! - nrf52810: Section 4.5
//! - nrf52811: Section 4.5
//! - nrf52832: Section 14
//! - nrf52840: Section 4.5
use crate::pac::{NVMC, UICR};

/// Interface to a UICR instance.
///
/// This is a very basic interface that comes with the following limitations:
/// - Only `customer` registers are usable for storing and loading of data
/// - Erase must be performed in order to write bits with value `1` over `0`
pub struct Uicr(UICR);

impl Uicr {
    /// Construct a new `Uicr` from `pac::UICR`.
    pub fn new(uicr: UICR) -> Self {
        Self(uicr)
    }

    /// Release the `pac::UICR` instance back.
    pub fn free(self) -> UICR {
        self.0
    }

    /// Erase the UICR registers.
    ///
    /// UICR registers can only be set to `0` bits, additional overrides back to `1` can only be
    /// performed by erasing the UICR registers.
    /// - Sets all registers to 0xFFFF_FFFFu32
    pub fn erase(&mut self, nvmc: &mut NVMC) {
        assert!(!nvmc.config.read().wen().is_wen()); // write + erase is forbidden!

        nvmc.config.write(|w| w.wen().een());
        nvmc.eraseuicr.write(|w| w.eraseuicr().erase());
        nvmc.config.reset()
    }

    /// Store a slice of `&[u32]` values to the customer registers with given offset.
    ///
    /// - offset + slice length must be less than 32
    /// - initial value after erase is 0xFFFF_FFFFu32
    /// - UICR registers can only be set to `0` bits, additional overrides back to `1` can only be
    ///   performed by erasing the UICR registers
    pub fn store_customer(&mut self, nvmc: &mut NVMC, offset: usize, values: &[u32]) {
        assert!(values.len() + offset <= self.0.customer.len()); // ensure we fit
        assert!(!nvmc.config.read().wen().is_een()); // write + erase is forbidden!

        nvmc.config.write(|w| w.wen().wen());
        for (i, value) in values.iter().enumerate() {
            #[cfg(feature = "51")]
            self.0.customer[offset + i].write(|w| unsafe { w.bits(*value) });

            #[cfg(not(feature = "51"))]
            self.0.customer[offset + i].write(|w| unsafe { w.customer().bits(*value) });
        }
        nvmc.config.reset()
    }

    /// Load a slice of `&[u32]` values to the customer registers from given offset.
    ///
    /// - offset + slice length must be less than 32
    /// - returns the loaded slice
    pub fn load_customer<'a>(&mut self, offset: usize, values: &'a mut [u32]) -> &'a [u32] {
        assert!(values.len() + offset <= self.0.customer.len()); // ensure we fit

        let range = offset..offset + values.len();
        for (i, reg_i) in range.enumerate() {
            #[cfg(feature = "51")]
            {
                values[i] = self.0.customer[reg_i].read().bits()
            }

            #[cfg(not(feature = "51"))]
            {
                values[i] = self.0.customer[reg_i].read().customer().bits()
            }
        }

        values
    }
}
