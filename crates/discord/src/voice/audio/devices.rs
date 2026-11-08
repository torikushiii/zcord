use super::*;

static MISSING_DEVICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    pub name: String,
    pub label: String,
    pub detail: Option<String>,
    pub kind: AudioDeviceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDeviceKind {
    Bluetooth,
    Display,
    Headphones,
    Microphone,
    Speaker,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
    pub default_input: Option<AudioDevice>,
    pub default_output: Option<AudioDevice>,
}

pub fn audio_devices() -> AudioDevices {
    let host = cpal::default_host();
    AudioDevices {
        inputs: host.input_devices().map(device_list).unwrap_or_default(),
        outputs: host.output_devices().map(device_list).unwrap_or_default(),
        default_input: host.default_input_device().as_ref().and_then(describe),
        default_output: host.default_output_device().as_ref().and_then(describe),
    }
}

fn device_list(devices: impl Iterator<Item = cpal::Device>) -> Vec<AudioDevice> {
    let mut list: Vec<AudioDevice> = Vec::new();
    for device in devices {
        let Some(device) = describe(&device) else {
            continue;
        };
        if !list.iter().any(|listed| listed.name == device.name) {
            list.push(device);
        }
    }
    list
}

fn describe(device: &cpal::Device) -> Option<AudioDevice> {
    let description = device.description().ok()?;
    let name = description.name().to_owned();
    let (label, detail) = split_name(&name);
    Some(AudioDevice {
        label,
        detail,
        kind: device_kind(description.interface_type(), description.device_type()),
        name,
    })
}

fn split_name(name: &str) -> (String, Option<String>) {
    let parts = name
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .filter(|(endpoint, adapter)| !endpoint.is_empty() && !adapter.is_empty());
    match parts {
        Some((endpoint, adapter)) => (adapter.to_owned(), Some(endpoint.to_owned())),
        None => (name.to_owned(), None),
    }
}

fn device_kind(interface: cpal::InterfaceType, kind: cpal::DeviceType) -> AudioDeviceKind {
    use cpal::{DeviceType, InterfaceType};
    match (interface, kind) {
        (InterfaceType::Bluetooth, _) => AudioDeviceKind::Bluetooth,
        (InterfaceType::Hdmi | InterfaceType::DisplayPort, _) => AudioDeviceKind::Display,
        (
            _,
            DeviceType::Headphones
            | DeviceType::Headset
            | DeviceType::Earpiece
            | DeviceType::Handset
            | DeviceType::HearingAid,
        ) => AudioDeviceKind::Headphones,
        (_, DeviceType::Microphone) => AudioDeviceKind::Microphone,
        (_, DeviceType::Speaker) => AudioDeviceKind::Speaker,
        _ => AudioDeviceKind::Other,
    }
}

pub(super) fn store_device(
    slot: &Mutex<Option<String>>,
    generation: &AtomicU64,
    name: Option<String>,
) {
    let mut current = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if *current == name {
        return;
    }
    *current = name;
    generation.fetch_add(1, Ordering::AcqRel);
}

pub(super) fn pick(names: &[String], wanted: Option<&str>) -> Option<usize> {
    let wanted = wanted?;
    names.iter().position(|name| name == wanted)
}

fn named_devices(devices: impl Iterator<Item = cpal::Device>) -> Vec<(cpal::Device, String)> {
    let mut named = Vec::new();
    for device in devices {
        if let Ok(description) = device.description() {
            named.push((device, description.name().to_owned()));
        }
    }
    named
}

fn matching_device(host: &cpal::Host, wanted: &str, input: bool) -> Option<cpal::Device> {
    let named = if input {
        named_devices(host.input_devices().ok()?)
    } else {
        named_devices(host.output_devices().ok()?)
    };
    let names: Vec<String> = named.iter().map(|(_, name)| name.clone()).collect();
    let index = pick(&names, Some(wanted))?;
    named.into_iter().nth(index).map(|(device, _)| device)
}

fn log_missing_device(label: &str, name: &str) {
    let mut missing = MISSING_DEVICES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if missing.iter().any(|existing| existing == name) {
        return;
    }
    missing.push(name.to_owned());
    log::info!("{label} device {name:?} not found; using default");
}

pub(super) fn preferred_device(
    host: &cpal::Host,
    wanted: Option<&str>,
    input: bool,
    log_missing: bool,
) -> Option<cpal::Device> {
    if let Some(wanted) = wanted
        && let Some(device) = matching_device(host, wanted, input)
    {
        return Some(device);
    }
    if let Some(wanted) = wanted
        && log_missing
    {
        log_missing_device(if input { "Microphone" } else { "Speakers" }, wanted);
    }
    if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
}

#[cfg(test)]
mod device_name_tests {
    use super::split_name;

    #[test]
    fn windows_names_split_into_adapter_and_endpoint() {
        assert_eq!(
            split_name("Headphones (UGREEN HiTune Max5c)"),
            (
                "UGREEN HiTune Max5c".to_owned(),
                Some("Headphones".to_owned())
            )
        );
        assert_eq!(
            split_name("24V2W1G5 (2- High Definition Audio Device)"),
            (
                "2- High Definition Audio Device".to_owned(),
                Some("24V2W1G5".to_owned())
            )
        );
        assert_eq!(
            split_name("Digital Audio (S/PDIF) (High Definition Audio Device)"),
            (
                "High Definition Audio Device".to_owned(),
                Some("Digital Audio (S/PDIF)".to_owned())
            )
        );
        assert_eq!(split_name("default"), ("default".to_owned(), None));
    }
}
