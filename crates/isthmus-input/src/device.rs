//! Keyboards and mice as the user sees them, built from HID services.
//!
//! macOS shows one physical keyboard as several HID event services (a
//! keyboard one and a consumer-control one, sometimes more), and a USB
//! receiver as a keyboard and a mouse at once. The platform layer reads each
//! service into a [`HidService`]; [`group_services`] turns those into
//! [`Device`]s with a stable [`DeviceId`] that survives a replug, a KVM
//! switch and a reboot, which a registry id does not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One `IOHIDServiceClient`, read once. Strings are empty when the service
/// does not publish them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HidService {
    /// Per boot and per enumeration; never persisted.
    pub registry_id: u64,
    /// The USB or Bluetooth vendor id, part of the stable device id.
    pub vendor_id: u32,
    /// The vendor's product id, part of the stable device id.
    pub product_id: u32,
    /// The USB port path; stable while the device stays on the same port.
    pub location_id: u32,
    /// The product name, which the device lists show.
    pub product: String,
    /// The manufacturer name; the public HID client does not see it.
    pub manufacturer: String,
    /// Tells twins apart across ports when present; often empty.
    pub serial_number: String,
    /// `USB`, `Bluetooth`, `Bluetooth Low Energy`, `SPI`, `FIFO`, ...
    pub transport: String,
    /// Part of the Mac itself: never moves, and always has a Mac layout.
    pub built_in: bool,
    /// The page of the usage the service is mainly for (1 Generic Desktop,
    /// 12 Consumer).
    pub primary_usage_page: u32,
    /// The usage within that page (6 Keyboard, 2 Mouse, 1 Pointer).
    pub primary_usage: u32,
    /// `IOHIDServiceClientConformsTo(GenericDesktop, Keyboard)`.
    pub is_keyboard: bool,
    /// Conforms to GenericDesktop Mouse or Pointer.
    pub is_pointer: bool,
    /// `HIDScrollAccelerationType`: `HIDTrackpadScrollAcceleration` marks a
    /// trackpad or Magic Mouse, which Isthmus leaves alone.
    pub scroll_acceleration_type: String,
    /// `HIDPointerAccelerationType`: linear pointer mode only works when this
    /// is `HIDMouseAcceleration`.
    pub pointer_acceleration_type: String,
}

/// A device id that survives replugging: transport, vendor, product, kind,
/// and the serial number, or the port when there is no serial.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

impl DeviceId {
    /// The id as settings store it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a device is to Isthmus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    /// Gets a key mapping and the keyboard tap's translations.
    Keyboard,
    /// A mouse with a wheel Isthmus may change.
    Mouse,
    /// A trackpad or Magic Mouse: shown, never changed.
    Trackpad,
}

/// Where a keyboard's modifiers sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    /// fn | control | option | command | space.
    Mac,
    /// Ctrl | Win | Alt | space.
    Pc,
}

/// How sure [`detect_layout`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// A guess the user should check (a KVM hides the real keyboard).
    Low,
    /// Usually right; the name or vendor id points one way.
    Medium,
    /// The device says what it is (built-in, Apple-made).
    High,
}

/// One keyboard or mouse, as the device lists show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// The key of this device's settings; see [`device_id`].
    pub id: DeviceId,
    /// Which page lists it, and what Isthmus may change on it.
    pub kind: DeviceKind,
    /// The product name, else the manufacturer, else `vid:pid` in hex.
    pub name: String,
    /// Shown beside generic names such as "USB Receiver".
    pub manufacturer: String,
    /// Shown in the device details, in hex.
    pub vendor_id: u32,
    /// Shown in the device details, in hex.
    pub product_id: u32,
    /// As the services publish it (`USB`, `Bluetooth`, ...).
    pub transport: String,
    /// Part of the Mac itself; listed first.
    pub built_in: bool,
    /// The registry ids of every service that belongs to it, now.
    pub services: Vec<u64>,
    /// For keyboards; mice report [`Layout::Pc`] and ignore it.
    pub detected_layout: Layout,
    pub layout_confidence: Confidence,
    /// It is a KVM switch's emulated keyboard or mouse, or a receiver
    /// shared through one: the physical device behind it is unknown.
    pub via_kvm: bool,
    /// A keyboard whose firmware has a Mac/Windows mode (a switch, or a key
    /// combination) that swaps the keys beside the space bar without telling
    /// macOS anything: same ids, same services. Its layout is the mode it is
    /// in, which only the user can say (see [`has_mode_switch`]).
    #[serde(default)]
    pub has_mode_switch: bool,
}

/// Groups services into devices. Services of one physical device share
/// vendor, product, location and serial; within that, keyboard and pointer
/// services become separate devices (a receiver is both). Services that are
/// neither, and the virtual devices of Karabiner-Elements (which only replay
/// the real keyboards Isthmus already lists, so mapping them would apply a
/// mapping twice), are left out. The result is sorted: built-in first, then
/// by name, then by id.
pub fn group_services(services: &[HidService]) -> Vec<Device> {
    // A BTreeMap keeps the grouping deterministic whatever order the HID
    // event system lists services in.
    let mut groups: BTreeMap<(u32, u32, u32, &str), Vec<&HidService>> = BTreeMap::new();
    for service in services
        .iter()
        .filter(|service| (service.is_keyboard || service.is_pointer) && !is_karabiner(service))
    {
        let key = (
            service.vendor_id,
            service.product_id,
            service.location_id,
            service.serial_number.trim(),
        );
        groups.entry(key).or_default().push(service);
    }
    let mut devices = Vec::new();
    for members in groups.values() {
        let keyboards: Vec<&HidService> = members
            .iter()
            .copied()
            .filter(|service| service.is_keyboard)
            .collect();
        devices.extend(build_device(DeviceKind::Keyboard, &keyboards));
        let pointers: Vec<&HidService> = members
            .iter()
            .copied()
            .filter(|service| service.is_pointer)
            .collect();
        let kind = if pointers.iter().any(|service| is_trackpad(service)) {
            DeviceKind::Trackpad
        } else {
            DeviceKind::Mouse
        };
        devices.extend(build_device(kind, &pointers));
    }
    devices.sort_by(|a, b| {
        b.built_in
            .cmp(&a.built_in)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    devices
}

/// One device of `kind` from the services of one physical device that serve
/// it; `None` when there are none. The description comes from the service
/// whose primary usage is the kind's own (the keyboard, not the consumer
/// control), with whatever it lacks filled in from the others.
fn build_device(kind: DeviceKind, members: &[&HidService]) -> Option<Device> {
    let primary = members
        .iter()
        .copied()
        .min_by_key(|service| (!has_primary_usage_of(kind, service), service.registry_id))?;
    let first_non_empty = |field: fn(&HidService) -> &str| -> String {
        std::iter::once(primary)
            .chain(members.iter().copied())
            .map(|service| field(service).trim())
            .find(|value| !value.is_empty())
            .unwrap_or_default()
            .to_owned()
    };
    let merged = HidService {
        product: first_non_empty(|service| &service.product),
        manufacturer: first_non_empty(|service| &service.manufacturer),
        serial_number: first_non_empty(|service| &service.serial_number),
        transport: first_non_empty(|service| &service.transport),
        built_in: members.iter().any(|service| service.built_in),
        ..primary.clone()
    };
    let (detected_layout, layout_confidence) = match kind {
        DeviceKind::Keyboard => detect_layout(&merged),
        // Mice have no modifier row; the value is there for the JSON shape.
        DeviceKind::Mouse | DeviceKind::Trackpad => (Layout::Pc, Confidence::Low),
    };
    let name = if !merged.product.is_empty() {
        merged.product.clone()
    } else if !merged.manufacturer.is_empty() {
        merged.manufacturer.clone()
    } else {
        // Ids are language-neutral, which an invented English name is not.
        format!("{:04x}:{:04x}", merged.vendor_id, merged.product_id)
    };
    let mut registry_ids: Vec<u64> = members.iter().map(|service| service.registry_id).collect();
    registry_ids.sort_unstable();
    registry_ids.dedup();
    Some(Device {
        id: device_id(kind, &merged),
        kind,
        name,
        manufacturer: merged.manufacturer.clone(),
        vendor_id: merged.vendor_id,
        product_id: merged.product_id,
        transport: merged.transport.clone(),
        built_in: merged.built_in,
        services: registry_ids,
        detected_layout,
        layout_confidence,
        via_kvm: members.iter().any(|service| is_kvm(service)),
        has_mode_switch: kind == DeviceKind::Keyboard && has_mode_switch(&merged),
    })
}

/// Is `service`'s primary usage the one a device of `kind` is named after:
/// GenericDesktop Keyboard for keyboards, Mouse or Pointer for the others?
fn has_primary_usage_of(kind: DeviceKind, service: &HidService) -> bool {
    let generic_desktop = service.primary_usage_page == usage_page::GENERIC_DESKTOP;
    match kind {
        DeviceKind::Keyboard => generic_desktop && service.primary_usage == gd_usage::KEYBOARD,
        DeviceKind::Mouse | DeviceKind::Trackpad => {
            generic_desktop && matches!(service.primary_usage, gd_usage::MOUSE | gd_usage::POINTER)
        }
    }
}

/// Is this pointer a trackpad or a Magic Mouse, whose scrolling and tracking
/// macOS already does well and Isthmus never changes? Their drivers publish
/// trackpad scroll acceleration; the names catch one reached through a
/// driver that does not.
fn is_trackpad(service: &HidService) -> bool {
    let product = service.product.to_ascii_lowercase();
    service.scroll_acceleration_type == TRACKPAD_SCROLL_ACCELERATION
        || product.contains("trackpad")
        || product.contains("magic mouse")
}

/// Is this one of Karabiner-Elements' virtual HID devices?
fn is_karabiner(service: &HidService) -> bool {
    service
        .manufacturer
        .trim()
        .eq_ignore_ascii_case(KARABINER_MANUFACTURER)
        || service.product.trim_start().starts_with("Karabiner")
}

/// The id a device of `kind` built from `service` gets:
/// `{kind}:{transport}:{vid}:{pid}:sn={serial}`, or `…:loc={location}` when
/// the service publishes no serial; built-in devices, which never move, are
/// `{kind}:builtin:{vid}:{pid}`. The transport is lowercased with spaces as
/// dashes (`bluetooth-low-energy`), `unknown` when it is not published.
pub fn device_id(kind: DeviceKind, service: &HidService) -> DeviceId {
    let kind = match kind {
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Mouse => "mouse",
        DeviceKind::Trackpad => "trackpad",
    };
    let (vendor, product) = (service.vendor_id, service.product_id);
    if service.built_in {
        return DeviceId(format!("{kind}:builtin:{vendor:04x}:{product:04x}"));
    }
    let transport = match service.transport.trim() {
        "" => "unknown".to_owned(),
        transport => transport.to_lowercase().replace(' ', "-"),
    };
    let place = match service.serial_number.trim() {
        "" => format!("loc={:08x}", service.location_id),
        serial => format!("sn={serial}"),
    };
    DeviceId(format!(
        "{kind}:{transport}:{vendor:04x}:{product:04x}:{place}"
    ))
}

/// Guesses the modifier row from what HID publishes; no property says it
/// outright. Built-in and Apple-made keyboards are Mac (high); Apple's
/// vendor id on another name (Keychron and other clones use it) is Mac
/// (medium); a name with "Mac" in it is Mac (medium); a KVM's emulated
/// keyboard is PC (low); everything else is PC (medium).
///
/// "Mac" counts as a word of the product or manufacturer name ("MX Keys
/// Mac", "K380 for Mac", "macOS"), so "Macro Pad" stays PC. The KVM's guess
/// is low because the keyboard behind the switch is unknown.
pub fn detect_layout(service: &HidService) -> (Layout, Confidence) {
    let apple_vendor = is_apple_vendor(service.vendor_id);
    if service.built_in || (apple_vendor && is_apple_made(service)) {
        (Layout::Mac, Confidence::High)
    } else if apple_vendor || names_mac(service) {
        (Layout::Mac, Confidence::Medium)
    } else if is_kvm(service) {
        (Layout::Pc, Confidence::Low)
    } else {
        (Layout::Pc, Confidence::Medium)
    }
}

/// Does this keyboard have a Mac/Windows mode in its firmware? Keyboards
/// that borrow Apple's vendor id without being Apple's (Keychron and other
/// clones do, to get macOS's Apple keyboard driver) all have one, and so do
/// the brands below whatever id they report. Flipping such a switch changes
/// nothing macOS can see (checked on a Keychron K8: same ids, same service,
/// no reconnect), so the mode is the user's to tell.
pub fn has_mode_switch(service: &HidService) -> bool {
    if service.built_in {
        return false;
    }
    if is_apple_vendor(service.vendor_id) {
        return !is_apple_made(service);
    }
    let name = format!("{} {}", service.manufacturer, service.product).to_ascii_lowercase();
    MODE_SWITCH_BRANDS.iter().any(|brand| name.contains(brand))
}

/// Keyboard names whose makers ship a Mac/Windows mode: a slider (Keychron,
/// Lofree, NuPhy, Akko, Epomaker, Royal Kludge, Keydous) or Logitech's
/// Fn+O / Fn+P layouts on its multi-device keyboards.
const MODE_SWITCH_BRANDS: &[&str] = &[
    "keychron",
    "lofree",
    "nuphy",
    "akko",
    "epomaker",
    "royal kludge",
    "keydous",
    "mx keys",
    "mx mechanical",
    "k380",
    "k780",
    "pop keys",
];

/// Does an Apple-vendor keyboard also carry Apple's own name? Clones that
/// borrow the vendor id keep their own names (the Keychron K8 calls itself
/// "Keychron K8" in both fields).
fn is_apple_made(service: &HidService) -> bool {
    let product = service.product.trim_start();
    service.manufacturer.trim_start().starts_with("Apple")
        || product.starts_with("Apple")
        || product.starts_with("Magic Keyboard")
}

/// Does the product or manufacturer name contain the word "Mac" or "macOS"?
fn names_mac(service: &HidService) -> bool {
    [&service.product, &service.manufacturer]
        .into_iter()
        .any(|name| {
            name.split(|c: char| !c.is_alphanumeric())
                .any(|word| word.eq_ignore_ascii_case("mac") || word.eq_ignore_ascii_case("macos"))
        })
}

/// Is this service a KVM switch's emulated HID device? TESmart switches
/// publish their brand as the manufacturer, others use WCH bridge chips
/// (vendor 0x1A86) or say "KVM" in a name.
pub fn is_kvm(service: &HidService) -> bool {
    service.manufacturer.trim().eq_ignore_ascii_case("TESmart")
        || service.vendor_id == WCH_VENDOR_ID
        || service.product.to_ascii_lowercase().contains("kvm")
        || service.manufacturer.to_ascii_lowercase().contains("kvm")
}

/// Whether `vendor_id` is one of Apple's: a keyboard reporting it is (or
/// poses as) an Apple keyboard, with a Mac modifier row and media keys on
/// F1–F12 unless `HIDFKeyMode` says otherwise. Apple's keyboards report the
/// USB id over USB and the Bluetooth SIG id over Bluetooth, so testing only
/// the USB id would take a Bluetooth Magic Keyboard for a PC keyboard.
pub(crate) fn is_apple_vendor(vendor_id: u32) -> bool {
    matches!(vendor_id, APPLE_VENDOR_ID | APPLE_BLUETOOTH_VENDOR_ID)
}

/// Apple's USB vendor id, also borrowed by Keychron and other clones so that
/// macOS gives them its keyboard driver.
const APPLE_VENDOR_ID: u32 = 0x05AC;

/// Apple's Bluetooth SIG company id, the vendor id its keyboards, mice and
/// trackpads report when connected over Bluetooth.
const APPLE_BLUETOOTH_VENDOR_ID: u32 = 0x004C;

/// WCH (QinHeng), whose USB bridge chips emulate the keyboard and mouse of
/// many KVM switches.
const WCH_VENDOR_ID: u32 = 0x1A86;

/// The `HIDScrollAccelerationType` of trackpads and the Magic Mouse.
const TRACKPAD_SCROLL_ACCELERATION: &str = "HIDTrackpadScrollAcceleration";

/// The manufacturer Karabiner-Elements' virtual HID devices publish.
const KARABINER_MANUFACTURER: &str = "pqrs.org";

/// HID usage pages, for reading a service's primary usage.
mod usage_page {
    /// Generic Desktop (keyboards, mice, pointers).
    pub(super) const GENERIC_DESKTOP: u32 = 0x01;
}

/// Generic Desktop usages.
mod gd_usage {
    /// A pointing device without its own buttons (a trackpad's pointer part).
    pub(super) const POINTER: u32 = 0x01;
    /// A mouse.
    pub(super) const MOUSE: u32 = 0x02;
    /// A keyboard (and, on many boards, their consumer-control service too).
    pub(super) const KEYBOARD: u32 = 0x06;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Keychron K8 on this desk: Apple's vendor id, no serial, and two
    /// services that both conform to Keyboard, the keyboard itself and its
    /// consumer-control service.
    fn keychron_keyboard() -> HidService {
        HidService {
            registry_id: 0x1_0000_13e4,
            vendor_id: 0x05ac,
            product_id: 0x024f,
            location_id: 0x0011_2000,
            product: "Keychron K8".into(),
            manufacturer: "Keychron K8".into(),
            transport: "USB".into(),
            primary_usage_page: 1,
            primary_usage: 6,
            is_keyboard: true,
            ..HidService::default()
        }
    }

    /// The Keychron's consumer-control service, which also conforms to
    /// Keyboard and so belongs to the same keyboard.
    fn keychron_consumer() -> HidService {
        HidService {
            registry_id: 0x1_0000_13dd,
            primary_usage_page: 12,
            primary_usage: 1,
            ..keychron_keyboard()
        }
    }

    /// The keyboard a TESmart HKS201-M24 KVM switch emulates.
    fn tesmart_keyboard() -> HidService {
        HidService {
            registry_id: 0x1_0000_0fd1,
            vendor_id: 0x1a86,
            product_id: 0xe010,
            location_id: 0x0011_1000,
            product: "HKS201-M24".into(),
            manufacturer: "TESmart".into(),
            transport: "USB".into(),
            primary_usage_page: 1,
            primary_usage: 6,
            is_keyboard: true,
            ..HidService::default()
        }
    }

    /// The keyboard half of a Logitech USB receiver, which has a serial.
    fn receiver_keyboard() -> HidService {
        HidService {
            registry_id: 0x1_0000_608a,
            vendor_id: 0x046d,
            product_id: 0xc54d,
            location_id: 0x0013_3000,
            product: "USB Receiver".into(),
            manufacturer: "Logitech".into(),
            serial_number: "386A344A3234".into(),
            transport: "USB".into(),
            primary_usage_page: 1,
            primary_usage: 6,
            is_keyboard: true,
            ..HidService::default()
        }
    }

    /// The mouse half of the same receiver.
    fn receiver_mouse() -> HidService {
        HidService {
            registry_id: 0x1_0000_6091,
            primary_usage: 2,
            is_keyboard: false,
            is_pointer: true,
            scroll_acceleration_type: "HIDMouseScrollAcceleration".into(),
            pointer_acceleration_type: "HIDMouseAcceleration".into(),
            ..receiver_keyboard()
        }
    }

    /// USB speakers: consumer control only, neither keyboard nor pointer.
    fn speakers() -> HidService {
        HidService {
            registry_id: 0x1_0000_60c4,
            vendor_id: 0x2717,
            product_id: 0x5086,
            location_id: 0x0013_4000,
            product: "Redmi \u{7535}\u{8111}\u{97f3}\u{7bb1}".into(),
            transport: "USB".into(),
            primary_usage_page: 12,
            primary_usage: 1,
            ..HidService::default()
        }
    }

    /// Every input-related service this Mac lists.
    fn this_desk() -> Vec<HidService> {
        vec![
            keychron_keyboard(),
            keychron_consumer(),
            tesmart_keyboard(),
            receiver_mouse(),
            receiver_keyboard(),
            speakers(),
        ]
    }

    /// A MacBook's own keyboard and trackpad: one physical device, built in.
    fn builtin_keyboard_and_trackpad() -> Vec<HidService> {
        let keyboard = HidService {
            registry_id: 0x1_0000_0a01,
            vendor_id: 0x05ac,
            product_id: 0x0342,
            product: "Apple Internal Keyboard / Trackpad".into(),
            manufacturer: "Apple Inc.".into(),
            transport: "FIFO".into(),
            built_in: true,
            primary_usage_page: 1,
            primary_usage: 6,
            is_keyboard: true,
            ..HidService::default()
        };
        let trackpad = HidService {
            registry_id: 0x1_0000_0a02,
            primary_usage: 2,
            is_keyboard: false,
            is_pointer: true,
            scroll_acceleration_type: TRACKPAD_SCROLL_ACCELERATION.into(),
            ..keyboard.clone()
        };
        vec![keyboard, trackpad]
    }

    /// The ids of `devices`, in order.
    fn ids(devices: &[Device]) -> Vec<&str> {
        devices.iter().map(|device| device.id.as_str()).collect()
    }

    /// This desk's services become one device per physical keyboard and one
    /// per mouse: the Keychron's two services are one keyboard, the receiver
    /// is a keyboard and a mouse, and the speakers are not listed. Sorted by
    /// name, then id.
    #[test]
    fn this_desk_groups_into_one_device_per_keyboard_and_mouse() {
        let devices = group_services(&this_desk());
        assert_eq!(
            ids(&devices),
            [
                "keyboard:usb:1a86:e010:loc=00111000",
                "keyboard:usb:05ac:024f:loc=00112000",
                "keyboard:usb:046d:c54d:sn=386A344A3234",
                "mouse:usb:046d:c54d:sn=386A344A3234",
            ]
        );
        let keychron = &devices[1];
        assert_eq!(keychron.kind, DeviceKind::Keyboard);
        assert_eq!(keychron.name, "Keychron K8");
        assert_eq!(keychron.services, [0x1_0000_13dd, 0x1_0000_13e4]);
        let receiver_mouse = &devices[3];
        assert_eq!(receiver_mouse.kind, DeviceKind::Mouse);
        assert_eq!(receiver_mouse.services, [0x1_0000_6091]);
        assert_eq!(receiver_mouse.manufacturer, "Logitech");
    }

    /// The layout guess for each keyboard here: the Keychron borrows Apple's
    /// vendor id (Mac, medium), the KVM's emulated keyboard is PC with low
    /// confidence and marked as a KVM, and the receiver is PC.
    #[test]
    fn this_desk_gets_its_layout_guesses_and_kvm_marks() {
        let devices = group_services(&this_desk());
        let guess = |index: usize| {
            let device: &Device = &devices[index];
            (
                device.detected_layout,
                device.layout_confidence,
                device.via_kvm,
            )
        };
        assert_eq!(guess(0), (Layout::Pc, Confidence::Low, true));
        assert_eq!(guess(1), (Layout::Mac, Confidence::Medium, false));
        assert_eq!(guess(2), (Layout::Pc, Confidence::Medium, false));
        assert!(!guess(3).2);
    }

    /// Registry ids change on every replug and KVM switch; device ids do not.
    /// A device with a serial keeps its id on another port too, one without
    /// a serial is told apart by its port.
    #[test]
    fn device_ids_survive_a_replug_and_a_serial_survives_a_port_change() {
        let before = group_services(&this_desk());
        let replugged: Vec<HidService> = this_desk()
            .into_iter()
            .map(|service| HidService {
                registry_id: service.registry_id + 0x1000,
                ..service
            })
            .collect();
        assert_eq!(ids(&group_services(&replugged)), ids(&before));

        let moved = |service: HidService| HidService {
            location_id: 0x0014_1000,
            ..service
        };
        let receiver = device_id(DeviceKind::Mouse, &moved(receiver_mouse()));
        assert_eq!(receiver, device_id(DeviceKind::Mouse, &receiver_mouse()));
        let keychron = device_id(DeviceKind::Keyboard, &moved(keychron_keyboard()));
        assert_eq!(keychron.as_str(), "keyboard:usb:05ac:024f:loc=00141000");
    }

    /// Two identical keyboards without serials on two ports are two devices.
    #[test]
    fn twin_keyboards_on_two_ports_are_two_devices() {
        let other_port = HidService {
            registry_id: 0x1_0000_2000,
            location_id: 0x0011_3000,
            ..keychron_keyboard()
        };
        let devices = group_services(&[keychron_keyboard(), other_port]);
        assert_eq!(
            ids(&devices),
            [
                "keyboard:usb:05ac:024f:loc=00112000",
                "keyboard:usb:05ac:024f:loc=00113000",
            ]
        );
    }

    /// Built-in devices come first and carry no port or serial in their id;
    /// a pointer with trackpad scrolling is a trackpad, as is a Magic Mouse
    /// whatever its driver publishes; transports become lowercase slugs.
    #[test]
    fn builtin_devices_come_first_and_trackpads_are_told_from_mice() {
        let magic_mouse = HidService {
            registry_id: 0x1_0000_7000,
            vendor_id: 0x004c,
            product_id: 0x0269,
            product: "Magic Mouse".into(),
            serial_number: "aa-bb-cc-dd-ee-ff".into(),
            transport: "Bluetooth Low Energy".into(),
            primary_usage_page: 1,
            primary_usage: 2,
            is_pointer: true,
            ..HidService::default()
        };
        let mut services = this_desk();
        services.push(magic_mouse);
        services.extend(builtin_keyboard_and_trackpad());

        let devices = group_services(&services);
        let summary: Vec<(&str, DeviceKind)> = devices
            .iter()
            .map(|device| (device.id.as_str(), device.kind))
            .collect();
        assert_eq!(
            summary[0],
            ("keyboard:builtin:05ac:0342", DeviceKind::Keyboard)
        );
        assert_eq!(
            summary[1],
            ("trackpad:builtin:05ac:0342", DeviceKind::Trackpad)
        );
        assert!(summary.contains(&(
            "trackpad:bluetooth-low-energy:004c:0269:sn=aa-bb-cc-dd-ee-ff",
            DeviceKind::Trackpad
        )));
        assert_eq!(
            (devices[0].detected_layout, devices[0].layout_confidence),
            (Layout::Mac, Confidence::High)
        );
    }

    /// Karabiner-Elements' virtual keyboard and pointer are never listed:
    /// they replay the real devices, which are listed themselves.
    #[test]
    fn karabiner_virtual_devices_are_left_out() {
        let virtual_keyboard = HidService {
            registry_id: 0x1_0000_8000,
            vendor_id: 0x16c0,
            product_id: 0x27db,
            product: "Karabiner DriverKit VirtualHIDKeyboard 1.8.0".into(),
            manufacturer: "pqrs.org".into(),
            primary_usage_page: 1,
            primary_usage: 6,
            is_keyboard: true,
            ..HidService::default()
        };
        let virtual_pointer = HidService {
            registry_id: 0x1_0000_8001,
            product: "Karabiner DriverKit VirtualHIDPointing".into(),
            manufacturer: String::new(),
            primary_usage: 2,
            is_keyboard: false,
            is_pointer: true,
            ..virtual_keyboard.clone()
        };
        let devices = group_services(&[virtual_keyboard, virtual_pointer, keychron_keyboard()]);
        assert_eq!(ids(&devices), ["keyboard:usb:05ac:024f:loc=00112000"]);
    }

    /// A device's name falls back to the manufacturer, then to its ids, and
    /// a missing transport reads as `unknown` rather than an empty segment.
    #[test]
    fn a_nameless_device_is_named_by_manufacturer_or_ids() {
        let anonymous = HidService {
            product: String::new(),
            manufacturer: String::new(),
            transport: String::new(),
            ..tesmart_keyboard()
        };
        let devices = group_services(std::slice::from_ref(&anonymous));
        assert_eq!(devices[0].name, "1a86:e010");
        assert_eq!(
            devices[0].id.as_str(),
            "keyboard:unknown:1a86:e010:loc=00111000"
        );
        let by_maker = HidService {
            manufacturer: "TESmart".into(),
            ..anonymous
        };
        assert_eq!(group_services(&[by_maker])[0].name, "TESmart");
    }

    /// The layout heuristic, rule by rule: Apple-made is Mac with high
    /// confidence, a borrowed Apple vendor id or a "Mac" in the name is Mac
    /// with medium, a KVM is PC with low, anything else PC with medium.
    #[test]
    fn layout_detection_follows_the_heuristic_in_order() {
        let logitech = receiver_keyboard();
        let named = |product: &str| HidService {
            product: product.into(),
            ..logitech.clone()
        };
        let magic_keyboard = HidService {
            product: "Magic Keyboard with Touch ID".into(),
            manufacturer: "Apple Inc.".into(),
            ..keychron_keyboard()
        };
        assert_eq!(
            detect_layout(&magic_keyboard),
            (Layout::Mac, Confidence::High)
        );
        assert_eq!(
            detect_layout(&keychron_keyboard()),
            (Layout::Mac, Confidence::Medium)
        );
        assert_eq!(
            detect_layout(&named("MX Keys Mac")),
            (Layout::Mac, Confidence::Medium)
        );
        assert_eq!(
            detect_layout(&named("K380 for macOS")),
            (Layout::Mac, Confidence::Medium)
        );
        assert_eq!(
            detect_layout(&named("Macro Pad")),
            (Layout::Pc, Confidence::Medium)
        );
        assert_eq!(
            detect_layout(&tesmart_keyboard()),
            (Layout::Pc, Confidence::Low)
        );
        assert_eq!(detect_layout(&logitech), (Layout::Pc, Confidence::Medium));
    }

    /// A Magic Keyboard over Bluetooth reports Apple's Bluetooth vendor id
    /// (0x004C), not the USB one, and is still an Apple keyboard: Mac with
    /// high confidence, never taken for a PC keyboard.
    #[test]
    fn a_bluetooth_magic_keyboard_is_an_apple_keyboard() {
        let bluetooth_magic_keyboard = HidService {
            registry_id: 0x1_0000_7100,
            vendor_id: 0x004c,
            product_id: 0x0267,
            location_id: 0,
            product: "Magic Keyboard".into(),
            manufacturer: String::new(),
            serial_number: "aa-bb-cc-dd-ee-01".into(),
            transport: "Bluetooth".into(),
            ..keychron_keyboard()
        };
        assert!(is_apple_vendor(bluetooth_magic_keyboard.vendor_id));
        assert_eq!(
            detect_layout(&bluetooth_magic_keyboard),
            (Layout::Mac, Confidence::High)
        );
        let unnamed = HidService {
            product: "Keyboard".into(),
            ..bluetooth_magic_keyboard
        };
        assert_eq!(detect_layout(&unnamed), (Layout::Mac, Confidence::Medium));
        assert!(!is_apple_vendor(receiver_keyboard().vendor_id));
    }

    /// A KVM is recognised by the TESmart brand, a WCH bridge chip's vendor
    /// id, or "KVM" in a name; an ordinary receiver is not one.
    #[test]
    fn kvm_switches_are_recognised_by_brand_bridge_chip_or_name() {
        assert!(is_kvm(&tesmart_keyboard()));
        let bridge = HidService {
            manufacturer: String::new(),
            product: "USB HID".into(),
            ..tesmart_keyboard()
        };
        assert!(is_kvm(&bridge));
        let named = HidService {
            product: "4-Port KVM Switch".into(),
            ..receiver_keyboard()
        };
        assert!(is_kvm(&named));
        assert!(!is_kvm(&receiver_keyboard()));
        assert!(!is_kvm(&keychron_keyboard()));
    }

    /// Keyboards whose firmware swaps the keys beside the space bar are
    /// marked: Apple's id on another name, or a known brand; Apple's own and
    /// built-in keyboards, and plain PC keyboards, are not.
    #[test]
    fn keyboards_with_a_mode_switch_are_marked() {
        let keychron = HidService {
            vendor_id: 0x05ac,
            product_id: 0x024f,
            product: "Keychron K8".into(),
            manufacturer: "Keychron K8".into(),
            is_keyboard: true,
            ..HidService::default()
        };
        assert!(has_mode_switch(&keychron));
        let magic = HidService {
            vendor_id: 0x004c,
            product: "Magic Keyboard".into(),
            manufacturer: "Apple Inc.".into(),
            is_keyboard: true,
            ..HidService::default()
        };
        assert!(!has_mode_switch(&magic));
        let built_in = HidService {
            built_in: true,
            vendor_id: 0x05ac,
            product: "Keychron-like".into(),
            ..HidService::default()
        };
        assert!(!has_mode_switch(&built_in));
        let nuphy = HidService {
            vendor_id: 0x19f5,
            product: "NuPhy Air75 V2".into(),
            ..HidService::default()
        };
        assert!(has_mode_switch(&nuphy));
        let tesmart = HidService {
            vendor_id: 0x1a86,
            product: "HKS201-M24".into(),
            manufacturer: "TESmart".into(),
            ..HidService::default()
        };
        assert!(!has_mode_switch(&tesmart));
        let devices = group_services(&[HidService {
            registry_id: 1,
            is_keyboard: true,
            primary_usage_page: 1,
            primary_usage: 6,
            transport: "USB".into(),
            ..keychron
        }]);
        assert!(devices.iter().all(|d| d.has_mode_switch));
    }
}
