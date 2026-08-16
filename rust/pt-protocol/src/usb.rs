// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! USB P-touch printers over bulk endpoints (rusb / vendored libusb).

use std::time::Duration;

use crate::{Error, ModelSpec, Result, Transport, PT18R, PTH500};

const VENDOR_ID: u16 = 0x04F9;
/// USB product id → model, one entry per supported USB P-touch. Both
/// use the same interface shape: printer class, bulk OUT 0x02, bulk
/// IN 0x81 (H500 per its raster reference; 18R probed on hardware).
const MODELS: &[(u16, &ModelSpec)] = &[(0x205E, &PTH500), (0x201A, &PT18R)];
const EP_OUT: u8 = 0x02;
const EP_IN: u8 = 0x81;

pub struct UsbTransport {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
}

/// Is `id` the name of a USB model this build supports?
pub fn is_usb_model(id: &str) -> bool {
    MODELS.iter().any(|(_, spec)| spec.name == id)
}

/// The supported models currently enumerated on the bus, in MODELS
/// order, deduplicated. Claims nothing.
pub fn present_models() -> Vec<&'static ModelSpec> {
    let Ok(devices) = rusb::devices() else {
        return Vec::new();
    };
    let present: Vec<u16> = devices
        .iter()
        .filter_map(|d| d.device_descriptor().ok())
        .filter(|desc| desc.vendor_id() == VENDOR_ID)
        .map(|desc| desc.product_id())
        .collect();
    MODELS
        .iter()
        .filter(|(pid, _)| present.contains(pid))
        .map(|(_, spec)| *spec)
        .collect()
}

impl UsbTransport {
    /// Open the USB printer named `model` (an id from
    /// `available_printers`). With `None`, models are tried in MODELS
    /// order so the default is deterministic when several USB
    /// printers are plugged in (bus enumeration order is not).
    pub fn find(model: Option<&str>) -> Result<Option<(UsbTransport, &'static ModelSpec)>> {
        let devices = rusb::devices().map_err(|e| Error(format!("USB enumeration failed: {e}")))?;
        for (pid, spec) in MODELS {
            if model.is_some_and(|name| name != spec.name) {
                continue;
            }
            for device in devices.iter() {
                let Ok(desc) = device.device_descriptor() else {
                    continue;
                };
                if desc.vendor_id() != VENDOR_ID || desc.product_id() != *pid {
                    continue;
                }
                let handle = device.open().map_err(|e| match e {
                    rusb::Error::Access => {
                        Error("the printer interface is held by another process".into())
                    }
                    other => Error(format!("USB open failed: {other}")),
                })?;
                let _ = handle.set_auto_detach_kernel_driver(true);
                handle
                    .claim_interface(0)
                    .map_err(|e| Error(format!("USB claim failed: {e}")))?;
                return Ok(Some((UsbTransport { handle }, spec)));
            }
        }
        Ok(None)
    }
}

impl Transport for UsbTransport {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        let mut off = 0;
        while off < data.len() {
            // The printer NAKs while its buffer drains at mechanical
            // print speed, so a raster write can stall for seconds.
            let n = self
                .handle
                .write_bulk(EP_OUT, &data[off..], Duration::from_secs(30))
                .map_err(|e| Error(format!("USB write failed: {e}")))?;
            off += n;
        }
        Ok(())
    }

    fn read(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; max];
        match self
            .handle
            .read_bulk(EP_IN, &mut buf, Duration::from_millis(200))
        {
            Ok(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            Err(rusb::Error::Timeout) => Ok(Vec::new()),
            Err(e) => Err(Error(format!("USB read failed: {e}"))),
        }
    }

    fn drain(&mut self) -> Result<()> {
        while !self.read(4096)?.is_empty() {}
        Ok(())
    }
}
