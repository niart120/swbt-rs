use crate::error::{Error, ErrorKind};

/// Opaque selector passed to the Bluetooth adapter backend.
///
/// The selector syntax is interpreted by the concrete backend. This type keeps
/// backend-specific selector types out of the public controller API.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AdapterSelector(Box<str>);

impl From<String> for AdapterSelector {
    fn from(value: String) -> Self {
        Self(value.into_boxed_str())
    }
}

impl From<&str> for AdapterSelector {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

/// A Bluetooth HCI USB adapter found without opening or claiming the device.
///
/// String descriptors are intentionally not read during discovery because
/// doing so requires opening the USB device. [`Self::has_serial_number`]
/// reports only whether the device descriptor advertises a serial-number
/// string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterInfo {
    selector: AdapterSelector,
    vendor_id: u16,
    product_id: u16,
    bus_number: u8,
    port_numbers: Option<Box<[u8]>>,
    has_serial_number: bool,
}

impl AdapterInfo {
    #[cfg(any(feature = "bumble", test))]
    fn new(
        selector: impl Into<AdapterSelector>,
        vendor_id: u16,
        product_id: u16,
        bus_number: u8,
        port_numbers: Option<Box<[u8]>>,
        has_serial_number: bool,
    ) -> Self {
        Self {
            selector: selector.into(),
            vendor_id,
            product_id,
            bus_number,
            port_numbers,
            has_serial_number,
        }
    }

    /// Returns the `usb:N` selector for this discovery result.
    #[must_use]
    pub const fn selector(&self) -> &AdapterSelector {
        &self.selector
    }

    /// Returns the USB vendor identifier.
    #[must_use]
    pub const fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    /// Returns the USB product identifier.
    #[must_use]
    pub const fn product_id(&self) -> u16 {
        self.product_id
    }

    /// Returns the libusb bus number observed during discovery.
    #[must_use]
    pub const fn bus_number(&self) -> u8 {
        self.bus_number
    }

    /// Returns the USB topology path observed during discovery, when available.
    ///
    /// Descriptor-only discovery still returns the adapter when the platform
    /// cannot provide its port path.
    #[must_use]
    pub fn port_numbers(&self) -> Option<&[u8]> {
        self.port_numbers.as_deref()
    }

    /// Returns whether the device descriptor has a serial-number string index.
    ///
    /// This does not mean that the serial string was read or can be read
    /// without permission to open the device.
    #[must_use]
    pub const fn has_serial_number(&self) -> bool {
        self.has_serial_number
    }
}

/// Lists USB devices whose device or interface class identifies a Bluetooth
/// primary controller.
///
/// Discovery reads USB descriptors only. It does not open a device, detach a
/// kernel driver, claim an interface, or send an HCI command.
/// Individual devices with unreadable device or configuration descriptors are
/// omitted while the number omitted is emitted as a secret-free trace event.
/// Each returned `usb:N` selector is the zero-based index among matching HCI
/// candidates in libusb enumeration order.
///
/// # Errors
///
/// Returns [`ErrorKind::UnsupportedCapability`] when the crate was built
/// without the `bumble` feature. Returns [`ErrorKind::AdapterDiscovery`] when
/// the USB context or device list cannot be read.
pub fn list_adapters() -> crate::Result<Vec<AdapterInfo>> {
    #[cfg(feature = "bumble")]
    {
        discover_with_probe(&mut RusbDescriptorProbe)
    }

    #[cfg(not(feature = "bumble"))]
    {
        Err(Error::new(
            ErrorKind::UnsupportedCapability,
            "USB adapter discovery is unavailable in this build",
        ))
    }
}

#[cfg(any(feature = "bumble", test))]
const BLUETOOTH_HCI_CLASS: UsbClass = UsbClass::new(0xe0, 0x01, 0x01);

#[cfg(any(feature = "bumble", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsbClass {
    class: u8,
    subclass: u8,
    protocol: u8,
}

#[cfg(any(feature = "bumble", test))]
impl UsbClass {
    const fn new(class: u8, subclass: u8, protocol: u8) -> Self {
        Self {
            class,
            subclass,
            protocol,
        }
    }
}

#[cfg(any(feature = "bumble", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct UsbDescriptorRecord {
    vendor_id: u16,
    product_id: u16,
    bus_number: u8,
    port_numbers: Option<Box<[u8]>>,
    has_serial_number: bool,
    device_class: UsbClass,
    interface_classes: Box<[UsbClass]>,
}

#[cfg(any(feature = "bumble", test))]
impl UsbDescriptorRecord {
    fn new(
        vendor_id: u16,
        product_id: u16,
        bus_number: u8,
        port_numbers: Option<Vec<u8>>,
        has_serial_number: bool,
        device_class: UsbClass,
        interface_classes: Vec<UsbClass>,
    ) -> Self {
        Self {
            vendor_id,
            product_id,
            bus_number,
            port_numbers: port_numbers.map(Vec::into_boxed_slice),
            has_serial_number,
            device_class,
            interface_classes: interface_classes.into_boxed_slice(),
        }
    }

    fn is_bluetooth_hci(&self) -> bool {
        self.device_class == BLUETOOTH_HCI_CLASS
            || (self.device_class.class == 0
                && self.interface_classes.contains(&BLUETOOTH_HCI_CLASS))
    }
}

#[cfg(any(feature = "bumble", test))]
trait DescriptorProbe {
    type Error: std::error::Error + Send + Sync + 'static;

    fn descriptor_records(&mut self) -> Result<Vec<UsbDescriptorRecord>, Self::Error>;
}

#[cfg(any(feature = "bumble", test))]
fn discover_with_probe(probe: &mut impl DescriptorProbe) -> crate::Result<Vec<AdapterInfo>> {
    let records = probe.descriptor_records().map_err(|source| {
        Error::with_source(
            ErrorKind::AdapterDiscovery,
            "USB adapter descriptors could not be listed",
            source,
        )
    })?;

    Ok(records
        .into_iter()
        .filter(UsbDescriptorRecord::is_bluetooth_hci)
        .enumerate()
        .map(|(index, record)| {
            AdapterInfo::new(
                format!("usb:{index}"),
                record.vendor_id,
                record.product_id,
                record.bus_number,
                record.port_numbers,
                record.has_serial_number,
            )
        })
        .collect())
}

#[cfg(feature = "bumble")]
struct RusbDescriptorProbe;

#[cfg(feature = "bumble")]
impl DescriptorProbe for RusbDescriptorProbe {
    type Error = rusb::Error;

    fn descriptor_records(&mut self) -> Result<Vec<UsbDescriptorRecord>, Self::Error> {
        use rusb::UsbContext;

        let context = rusb::Context::new()?;
        let devices = context.devices()?;
        let mut records = Vec::new();
        let mut skipped_device_count = 0_u64;
        for device in devices.iter() {
            match descriptor_record(device) {
                Some(record) => records.push(record),
                None => skipped_device_count += 1,
            }
        }
        if skipped_device_count > 0 {
            tracing::debug!(
                skipped_device_count,
                "USB adapter discovery skipped devices with unreadable descriptors"
            );
        }
        Ok(records)
    }
}

#[cfg(feature = "bumble")]
fn descriptor_record(device: rusb::Device<rusb::Context>) -> Option<UsbDescriptorRecord> {
    let descriptor = device.device_descriptor().ok()?;
    let device_class = UsbClass::new(
        descriptor.class_code(),
        descriptor.sub_class_code(),
        descriptor.protocol_code(),
    );
    let mut interface_classes = Vec::new();
    if device_class.class == 0 {
        for config_index in 0..descriptor.num_configurations() {
            let configuration = device.config_descriptor(config_index).ok()?;
            for interface in configuration.interfaces() {
                interface_classes.extend(interface.descriptors().map(|setting| {
                    UsbClass::new(
                        setting.class_code(),
                        setting.sub_class_code(),
                        setting.protocol_code(),
                    )
                }));
            }
        }
    }

    Some(UsbDescriptorRecord::new(
        descriptor.vendor_id(),
        descriptor.product_id(),
        device.bus_number(),
        device.port_numbers().ok(),
        descriptor.serial_number_string_index().is_some(),
        device_class,
        interface_classes,
    ))
}

#[cfg(test)]
mod tests {
    use std::{error::Error as StdError, fmt};

    use super::{AdapterInfo, DescriptorProbe, UsbClass, UsbDescriptorRecord, discover_with_probe};

    const BLUETOOTH_HCI: UsbClass = UsbClass::new(0xe0, 0x01, 0x01);

    #[test]
    fn descriptor_discovery_returns_only_bluetooth_hci_without_opening_or_claiming() {
        let mut probe = FakeDescriptorProbe {
            records: vec![
                UsbDescriptorRecord::new(
                    0x0a12,
                    0x0001,
                    1,
                    Some(vec![4]),
                    true,
                    UsbClass::new(0, 0, 0),
                    vec![BLUETOOTH_HCI],
                ),
                UsbDescriptorRecord::new(
                    0x1234,
                    0xabcd,
                    1,
                    Some(vec![5]),
                    false,
                    UsbClass::new(0xff, 0, 0),
                    vec![],
                ),
                UsbDescriptorRecord::new(0x0489, 0xe13a, 2, None, false, BLUETOOTH_HCI, vec![]),
            ],
        };

        let adapters = discover_with_probe(&mut probe).expect("discover fake USB descriptors");

        assert_eq!(
            adapters,
            vec![
                AdapterInfo::new(
                    "usb:0",
                    0x0a12,
                    0x0001,
                    1,
                    Some(vec![4].into_boxed_slice()),
                    true,
                ),
                AdapterInfo::new("usb:1", 0x0489, 0xe13a, 2, None, false,),
            ]
        );
        assert_eq!(
            adapters[0].selector(),
            &super::AdapterSelector::from("usb:0")
        );
        assert_eq!(adapters[0].vendor_id(), 0x0a12);
        assert_eq!(adapters[0].product_id(), 0x0001);
        assert_eq!(adapters[0].bus_number(), 1);
        assert_eq!(adapters[0].port_numbers(), Some([4].as_slice()));
        assert_eq!(adapters[1].port_numbers(), None);
        assert!(adapters[0].has_serial_number());
    }

    #[test]
    fn descriptor_inventory_failure_keeps_a_typed_discovery_source() {
        let error = discover_with_probe(&mut FailingDescriptorProbe)
            .expect_err("descriptor inventory failure must be reported");

        assert_eq!(error.kind(), crate::ErrorKind::AdapterDiscovery);
        assert_eq!(
            error.source().expect("discovery source").to_string(),
            "fake descriptor probe failed"
        );
        assert!(!error.to_string().contains("fake"));
        assert!(!format!("{error:?}").contains("fake"));
    }

    struct FakeDescriptorProbe {
        records: Vec<UsbDescriptorRecord>,
    }

    impl DescriptorProbe for FakeDescriptorProbe {
        type Error = FakeProbeError;

        fn descriptor_records(&mut self) -> Result<Vec<UsbDescriptorRecord>, Self::Error> {
            Ok(self.records.clone())
        }
    }

    #[derive(Debug)]
    struct FakeProbeError;

    impl fmt::Display for FakeProbeError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fake descriptor probe failed")
        }
    }

    impl std::error::Error for FakeProbeError {}

    struct FailingDescriptorProbe;

    impl DescriptorProbe for FailingDescriptorProbe {
        type Error = FakeProbeError;

        fn descriptor_records(&mut self) -> Result<Vec<UsbDescriptorRecord>, Self::Error> {
            Err(FakeProbeError)
        }
    }
}
