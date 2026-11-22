//! `isthmus devices`: the keyboards, mice and trackpads as Isthmus sees
//! them, read once from the HID event system. Read-only and needs no
//! permission, so it is the first thing to run when a device is treated
//! wrongly: it shows the stable id the settings use, the layout detection
//! guessed, and whether the device sits behind a KVM switch.

use isthmus_input::device::{self, Confidence, Device, DeviceKind, Layout};
use isthmus_input::platform;
use snafu::ResultExt;

use crate::error::{CliError, ListDevicesSnafu};
use crate::output;

/// Lists the devices, as a table or as JSON.
pub(crate) fn run(json: bool) -> Result<(), CliError> {
    let services = platform::list_services().context(ListDevicesSnafu)?;
    let devices = device::group_services(&services);
    if json {
        return output::print_json(&devices);
    }
    if devices.is_empty() {
        println!("No keyboards, mice or trackpads found.");
        return Ok(());
    }
    print!("{}", table(&devices));
    Ok(())
}

/// The columns, in the order [`row`] fills them.
const HEADER: [&str; 8] = [
    "KIND",
    "NAME",
    "MANUFACTURER",
    "VID:PID",
    "TRANSPORT",
    "LAYOUT",
    "KVM",
    "ID",
];

/// One row per device, columns padded to the widest cell.
fn table(devices: &[Device]) -> String {
    let rows: Vec<[String; 8]> = devices.iter().map(row).collect();
    let mut widths = HEADER.map(|title| title.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut text = String::new();
    let header = HEADER.map(str::to_owned);
    for cells in std::iter::once(&header).chain(&rows) {
        let mut line = String::new();
        for (cell, width) in cells.iter().zip(widths) {
            let pad = width.saturating_sub(cell.chars().count());
            line.push_str(cell);
            line.push_str(&" ".repeat(pad + 2));
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

/// A device's cells. The layout means something only for a keyboard: mice
/// report PC and ignore it.
fn row(device: &Device) -> [String; 8] {
    let kind = match device.kind {
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Mouse => "mouse",
        DeviceKind::Trackpad => "trackpad",
    };
    let layout = match device.kind {
        DeviceKind::Keyboard => format!(
            "{} ({})",
            layout_name(device.detected_layout),
            confidence_name(device.layout_confidence)
        ),
        DeviceKind::Mouse | DeviceKind::Trackpad => "-".to_owned(),
    };
    let transport = match (device.transport.is_empty(), device.built_in) {
        (true, true) => "built-in".to_owned(),
        (true, false) => "-".to_owned(),
        (false, true) => format!("{}, built-in", device.transport),
        (false, false) => device.transport.clone(),
    };
    [
        kind.to_owned(),
        or_dash(&device.name),
        or_dash(&device.manufacturer),
        format!("{:04x}:{:04x}", device.vendor_id, device.product_id),
        transport,
        layout,
        if device.via_kvm { "yes" } else { "-" }.to_owned(),
        device.id.to_string(),
    ]
}

fn layout_name(layout: Layout) -> &'static str {
    match layout {
        Layout::Mac => "mac",
        Layout::Pc => "pc",
    }
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

fn or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use isthmus_input::device::DeviceId;

    use super::*;

    fn device(kind: DeviceKind, name: &str, via_kvm: bool) -> Device {
        Device {
            id: DeviceId(format!("usb:1a86:e321:{name}")),
            kind,
            name: name.to_owned(),
            manufacturer: String::new(),
            vendor_id: 0x1a86,
            product_id: 0xe321,
            transport: "USB".to_owned(),
            built_in: false,
            services: vec![1],
            detected_layout: Layout::Pc,
            layout_confidence: Confidence::Low,
            via_kvm,
        }
    }

    /// The layout guess is shown for keyboards only, the KVM flag where it
    /// is set, and the id the settings key on, in full.
    #[test]
    fn rows_show_what_matters_for_each_kind() {
        let keyboard = row(&device(DeviceKind::Keyboard, "HKS201", true));
        assert_eq!(keyboard[3], "1a86:e321");
        assert_eq!(keyboard[5], "pc (low)");
        assert_eq!(keyboard[6], "yes");
        assert_eq!(keyboard[7], "usb:1a86:e321:HKS201");
        assert_eq!(keyboard[2], "-", "an unknown manufacturer is a dash");

        let mouse = row(&device(DeviceKind::Mouse, "G502", false));
        assert_eq!(mouse[5], "-");
        assert_eq!(mouse[6], "-");
    }

    /// Every line of the table starts its columns at the same offsets, so
    /// long names do not shift the ids.
    #[test]
    fn columns_line_up_whatever_the_name_length() {
        let text = table(&[
            device(DeviceKind::Keyboard, "K8", false),
            device(DeviceKind::Mouse, "A much longer mouse name", false),
        ]);
        let offsets: Vec<usize> = text
            .lines()
            .map(|line| {
                line.find("1a86:e321")
                    .unwrap_or_else(|| line.find("VID").unwrap())
            })
            .collect();
        assert_eq!(offsets.len(), 3);
        assert!(offsets.iter().all(|&offset| offset == offsets[0]), "{text}");
    }
}
