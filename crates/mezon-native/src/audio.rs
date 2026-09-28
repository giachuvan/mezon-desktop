use cpal::Sample as _;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use flume::Sender;
use std::sync::Mutex;

#[cfg(target_os = "windows")]
pub const WINDOWS_COMMUNICATIONS_INPUT_DEVICE_ID: &str = "mezon:audio-input:windows-communications";

/// Describes a detected audio device.
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    /// Stable persistent device identifier from `cpal::Device::id()`.
    /// Survives reboots and reconnects.
    pub id: String,
    /// Human-readable name reported by the OS (`cpal::Device::description()`).
    pub name: String,
}

pub struct AudioDeviceSnapshot {
    pub inputs: Vec<AudioDeviceInfo>,
    pub outputs: Vec<AudioDeviceInfo>,
    pub default_input_name: Option<String>,
    pub default_output_name: Option<String>,
}

pub fn audio_device_snapshot() -> AudioDeviceSnapshot {
    #[cfg(target_os = "linux")]
    {
        linux_audio_device_snapshot()
    }
    #[cfg(not(target_os = "linux"))]
    {
        AudioDeviceSnapshot {
            inputs: enumerate_input_devices(),
            outputs: enumerate_output_devices(),
            default_input_name: default_input_device_name(),
            default_output_name: default_output_device_name(),
        }
    }
}

/// Enumerate all available audio input (microphone) devices.
///
/// Returns an empty vec on error or when no devices are found.
pub fn enumerate_input_devices() -> Vec<AudioDeviceInfo> {
    #[cfg(target_os = "linux")]
    {
        linux_audio_device_snapshot().inputs
    }
    #[cfg(not(target_os = "linux"))]
    {
        let host = cpal::default_host();
        let Ok(devices) = host.input_devices() else {
            tracing::warn!("Failed to enumerate input devices");
            return vec![];
        };
        let devices = collect_devices(devices);
        #[cfg(target_os = "windows")]
        {
            let mut devices = devices;
            append_windows_communications_input(&mut devices);
            devices
        }
        #[cfg(not(target_os = "windows"))]
        {
            devices
        }
    }
}

/// Enumerate all available audio output (speaker/headphone) devices.
///
/// Returns an empty vec on error or when no devices are found.
pub fn enumerate_output_devices() -> Vec<AudioDeviceInfo> {
    #[cfg(target_os = "linux")]
    {
        linux_audio_device_snapshot().outputs
    }
    #[cfg(not(target_os = "linux"))]
    {
        let host = cpal::default_host();
        let Ok(devices) = host.output_devices() else {
            tracing::warn!("Failed to enumerate output devices");
            return vec![];
        };
        collect_devices(devices)
    }
}

pub fn default_input_device_name() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux_audio_device_snapshot().default_input_name
    }
    #[cfg(not(target_os = "linux"))]
    {
        host_default_device_name(true)
    }
}

pub fn default_output_device_name() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux_audio_device_snapshot().default_output_name
    }
    #[cfg(not(target_os = "linux"))]
    {
        host_default_device_name(false)
    }
}

#[cfg(not(target_os = "linux"))]
fn collect_devices(devices: impl Iterator<Item = cpal::Device>) -> Vec<AudioDeviceInfo> {
    let mut devices: Vec<AudioDeviceInfo> = devices
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let description = device.description().ok()?;
            let name = os_device_label(&description);
            Some(AudioDeviceInfo { id, name })
        })
        .collect();
    disambiguate_duplicate_names(&mut devices);
    devices
}

#[cfg(not(target_os = "linux"))]
fn host_default_device_name(capture: bool) -> Option<String> {
    let host = cpal::default_host();
    let device = if capture {
        host.default_input_device()?
    } else {
        host.default_output_device()?
    };
    let description = device.description().ok()?;
    Some(os_device_label(&description))
}

#[cfg(any(test, not(target_os = "linux")))]
fn os_device_label(description: &cpal::DeviceDescription) -> String {
    let name = description.name().trim();
    if let Some(friendly) = description
        .extended()
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        && (!is_generic_device_label(friendly) || is_generic_device_label(name))
    {
        return friendly.to_string();
    }
    if name.is_empty() {
        "Unknown device".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(any(test, not(target_os = "linux")))]
fn is_generic_device_label(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    matches!(
        name.as_str(),
        "usb audio"
            | "usb audio device"
            | "usb device"
            | "microphone"
            | "mic"
            | "speakers"
            | "speaker"
            | "headphones"
            | "headphone"
            | "headset"
            | "default"
            | "default audio device"
    ) || name.starts_with("usb device ")
        || name.starts_with("usb device 0x")
}

#[cfg(any(test, not(target_os = "linux")))]
fn disambiguate_duplicate_names(devices: &mut [AudioDeviceInfo]) {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for device in devices.iter() {
        *counts.entry(device.name.clone()).or_insert(0) += 1;
    }
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for device in devices.iter_mut() {
        if counts.get(&device.name).copied().unwrap_or(0) <= 1 {
            continue;
        }
        let next = seen.entry(device.name.clone()).or_insert(0);
        *next += 1;
        device.name = format!("{} ({next})", device.name);
    }
}

#[cfg(target_os = "windows")]
fn append_windows_communications_input(devices: &mut Vec<AudioDeviceInfo>) {
    let resolved = match resolve_input_device_id(WINDOWS_COMMUNICATIONS_INPUT_DEVICE_ID) {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!("Failed to resolve Windows communications microphone: {error}");
            return;
        }
    };
    let Some(device) = devices.iter().find(|device| device.id == resolved) else {
        tracing::warn!("Windows communications microphone is not in the active device list");
        return;
    };
    devices.push(AudioDeviceInfo {
        id: WINDOWS_COMMUNICATIONS_INPUT_DEVICE_ID.to_string(),
        name: format!("Communications - {}", device.name),
    });
}

#[cfg(target_os = "windows")]
pub fn resolve_input_device_id(device_id: &str) -> Result<String, String> {
    if device_id != WINDOWS_COMMUNICATIONS_INPUT_DEVICE_ID {
        return Ok(device_id.to_string());
    }
    windows_default_input_device_id(windows::Win32::Media::Audio::eCommunications)
}

#[cfg(target_os = "windows")]
fn windows_default_input_device_id(
    role: windows::Win32::Media::Audio::ERole,
) -> Result<String, String> {
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eCapture};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize,
    };

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
        return Err(format!("COM initialization failed: {initialized}"));
    }
    let _com = ComGuard(initialized.is_ok());

    let endpoint_id = unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| format!("MMDeviceEnumerator creation failed: {error}"))?;
        let endpoint = enumerator
            .GetDefaultAudioEndpoint(eCapture, role)
            .map_err(|error| format!("default capture endpoint lookup failed: {error}"))?;
        let id = endpoint
            .GetId()
            .map_err(|error| format!("capture endpoint ID lookup failed: {error}"))?;
        let result = id
            .to_string()
            .map_err(|error| format!("capture endpoint ID conversion failed: {error}"));
        CoTaskMemFree(Some(id.0.cast()));
        result?
    };

    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|error| format!("input device enumeration failed: {error}"))?;
    devices
        .filter_map(|device| device.id().ok())
        .find(|id| id.1 == endpoint_id)
        .map(|id| id.to_string())
        .ok_or_else(|| "default capture endpoint is not available through CPAL".to_string())
}

#[cfg(target_os = "linux")]
struct CardProfile {
    index: u32,
    id: String,
    model: Option<String>,
    distinguisher: Option<String>,
}

#[cfg(target_os = "linux")]
struct UsbIdentity {
    vendor: String,
    product: String,
    product_string: Option<String>,
    manufacturer: Option<String>,
    serial: Option<String>,
    port: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
struct PcmCandidate {
    rank: u8,
    card: u32,
    dev: u32,
    id: String,
    alsa_line: String,
    digital: bool,
}

#[cfg(target_os = "linux")]
struct PwNodeLabel {
    name: String,
    card: u32,
    device: u32,
    capture: bool,
    description: String,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct PwCatalog {
    nodes: Vec<PwNodeLabel>,
    card_descriptions: std::collections::HashMap<u32, String>,
    default_source_name: Option<String>,
    default_sink_name: Option<String>,
}

#[cfg(target_os = "linux")]
fn linux_audio_device_snapshot() -> AudioDeviceSnapshot {
    let cards = load_card_profiles();
    let pipewire = load_pipewire_catalog();
    let host = cpal::default_host();
    let inputs = match host.input_devices() {
        Ok(devices) => name_linux_devices(devices, true, &cards, &pipewire),
        Err(_) => {
            tracing::warn!("Failed to enumerate input devices");
            Vec::new()
        }
    };
    let outputs = match host.output_devices() {
        Ok(devices) => name_linux_devices(devices, false, &cards, &pipewire),
        Err(_) => {
            tracing::warn!("Failed to enumerate output devices");
            Vec::new()
        }
    };
    AudioDeviceSnapshot {
        inputs,
        outputs,
        default_input_name: pipewire_default_name(&pipewire, true)
            .or_else(|| host_default_device_name(true)),
        default_output_name: pipewire_default_name(&pipewire, false)
            .or_else(|| host_default_device_name(false)),
    }
}

#[cfg(target_os = "linux")]
fn name_linux_devices(
    devices: impl Iterator<Item = cpal::Device>,
    capture: bool,
    cards: &[CardProfile],
    pipewire: &PwCatalog,
) -> Vec<AudioDeviceInfo> {
    let mut candidates = Vec::new();
    for device in devices {
        let (Ok(id), Ok(description)) = (device.id(), device.description()) else {
            continue;
        };
        let pcm_id = description.driver().unwrap_or("");
        if !pcm_id.contains("CARD=") {
            continue;
        }
        let Some(token) = pcm_card_token(pcm_id) else {
            continue;
        };
        candidates.push(PcmCandidate {
            rank: alsa_pcm_rank(pcm_id),
            card: card_index_for(token, cards),
            dev: pcm_dev_index(pcm_id),
            id: id.to_string(),
            alsa_line: description.name().trim().to_string(),
            digital: alsa_pcm_is_digital_output(pcm_id),
        });
    }
    let selected = select_pcm_candidates(candidates);
    let mut named = assign_display_names(&selected, capture, cards, pipewire);
    named.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then(a.id.cmp(&b.id))
    });
    named
}

#[cfg(target_os = "linux")]
fn alsa_pcm_is_digital_output(pcm_id: &str) -> bool {
    pcm_id.starts_with("hdmi:") || pcm_id.starts_with("iec958:")
}

#[cfg(target_os = "linux")]
fn alsa_pcm_rank(pcm_id: &str) -> u8 {
    if pcm_id.starts_with("sysdefault:") {
        0
    } else if pcm_id.starts_with("plughw:") {
        1
    } else if pcm_id.starts_with("front:") {
        2
    } else if pcm_id.starts_with("hw:") {
        3
    } else {
        4
    }
}

#[cfg(target_os = "linux")]
fn pcm_card_token(pcm_id: &str) -> Option<&str> {
    let rest = pcm_id.split_once("CARD=")?.1;
    let token = rest.split([',', ' ', '\n']).next()?.trim();
    if token.is_empty() { None } else { Some(token) }
}

#[cfg(target_os = "linux")]
fn pcm_dev_index(pcm_id: &str) -> u32 {
    pcm_id
        .split_once("DEV=")
        .and_then(|(_, rest)| rest.split([',', ' ', '\n']).next())
        .and_then(|token| token.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn card_index_for(token: &str, cards: &[CardProfile]) -> u32 {
    if let Ok(index) = token.parse::<u32>()
        && cards.iter().any(|card| card.index == index)
    {
        return index;
    }
    if let Some(card) = cards.iter().find(|card| card.id == token) {
        return card.index;
    }
    if cards.is_empty()
        && let Ok(index) = token.parse::<u32>()
    {
        return index;
    }
    unresolved_card_key(token)
}

#[cfg(target_os = "linux")]
fn unresolved_card_key(token: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in token.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash | 0x8000_0000
}

#[cfg(target_os = "linux")]
fn select_pcm_candidates(mut items: Vec<PcmCandidate>) -> Vec<PcmCandidate> {
    let analog: Vec<PcmCandidate> = items.iter().filter(|item| !item.digital).cloned().collect();
    if !analog.is_empty() {
        items = analog;
    }
    items.sort_by(|a, b| {
        a.card
            .cmp(&b.card)
            .then(a.dev.cmp(&b.dev))
            .then(a.rank.cmp(&b.rank))
            .then(a.id.cmp(&b.id))
    });
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert((item.card, item.dev)));
    items
}

#[cfg(target_os = "linux")]
struct NamedPcm {
    id: String,
    card: u32,
    label: String,
    distinguisher: Option<String>,
}

#[cfg(target_os = "linux")]
fn assign_display_names(
    items: &[PcmCandidate],
    capture: bool,
    cards: &[CardProfile],
    pipewire: &PwCatalog,
) -> Vec<AudioDeviceInfo> {
    let named: Vec<NamedPcm> = items
        .iter()
        .map(|item| {
            let card = cards.iter().find(|card| card.index == item.card);
            let (label, distinguisher) = display_label(item, capture, card, pipewire);
            NamedPcm {
                id: item.id.clone(),
                card: item.card,
                label,
                distinguisher,
            }
        })
        .collect();
    disambiguate_labels(named, cards)
}

#[cfg(target_os = "linux")]
fn display_label(
    item: &PcmCandidate,
    capture: bool,
    card: Option<&CardProfile>,
    pipewire: &PwCatalog,
) -> (String, Option<String>) {
    let distinguisher = card.and_then(|card| card.distinguisher.clone());
    if let Some(node) = pipewire
        .nodes
        .iter()
        .find(|node| node.card == item.card && node.device == item.dev && node.capture == capture)
    {
        return (node.description.clone(), distinguisher);
    }
    let role = pcm_role(&item.alsa_line).filter(|role| pcm_role_is_useful(role));
    if let Some(device_name) = pipewire.card_descriptions.get(&item.card) {
        if let Some(role) = role {
            return (format!("{device_name} — {role}"), distinguisher);
        }
        return (device_name.clone(), distinguisher);
    }
    if let Some(model) = card.and_then(|card| card.model.clone()) {
        if let Some(role) = role.filter(|role| !model.contains(role.as_str())) {
            return (format!("{model} — {role}"), distinguisher);
        }
        return (model, distinguisher);
    }
    let fallback = if item.alsa_line.is_empty() {
        format!("Card {} device {}", item.card, item.dev)
    } else {
        item.alsa_line.clone()
    };
    (fallback, distinguisher)
}

#[cfg(target_os = "linux")]
fn disambiguate_labels(mut named: Vec<NamedPcm>, cards: &[CardProfile]) -> Vec<AudioDeviceInfo> {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for device in &named {
        *counts.entry(device.label.clone()).or_insert(0) += 1;
    }
    for device in &mut named {
        if counts.get(&device.label).copied().unwrap_or(0) <= 1 {
            continue;
        }
        let extra = device.distinguisher.clone().or_else(|| {
            cards
                .iter()
                .find(|card| card.index == device.card)
                .map(|card| card.id.clone())
        });
        if let Some(extra) =
            extra.filter(|extra| !extra.is_empty() && !device.label.contains(extra.as_str()))
        {
            device.label = format!("{} ({extra})", device.label);
        }
    }
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for device in &named {
        *counts.entry(device.label.clone()).or_insert(0) += 1;
    }
    let mut seen = std::collections::HashMap::<String, usize>::new();
    named
        .into_iter()
        .map(|device| {
            let mut name = device.label;
            if counts.get(&name).copied().unwrap_or(0) > 1 {
                let next = seen.entry(name.clone()).or_insert(0);
                *next += 1;
                name = format!("{name} #{next}");
            }
            AudioDeviceInfo {
                id: device.id,
                name,
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn pcm_role(alsa_line: &str) -> Option<String> {
    alsa_line
        .split_once(", ")
        .map(|(_, role)| role.trim().to_string())
        .filter(|role| !role.is_empty())
}

#[cfg(target_os = "linux")]
fn pcm_role_is_useful(role: &str) -> bool {
    !role.trim().is_empty() && !is_generic_usb_label(role)
}

#[cfg(target_os = "linux")]
fn is_generic_usb_label(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    name == "usb audio"
        || name == "usb device"
        || name.starts_with("usb device ")
        || name.starts_with("usb device 0x")
}

#[cfg(target_os = "linux")]
fn is_placeholder_default_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "default" | "default audio device" | "sysdefault" | "pipewire" | "pulse"
    )
}

#[cfg(target_os = "linux")]
fn host_default_device_name(capture: bool) -> Option<String> {
    let host = cpal::default_host();
    let device = if capture {
        host.default_input_device()?
    } else {
        host.default_output_device()?
    };
    let description = device.description().ok()?;
    let name = description.name().trim();
    if name.is_empty() || is_placeholder_default_name(name) {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(target_os = "linux")]
fn pipewire_default_name(catalog: &PwCatalog, capture: bool) -> Option<String> {
    let wanted = if capture {
        catalog.default_source_name.as_deref()
    } else {
        catalog.default_sink_name.as_deref()
    }?;
    catalog
        .nodes
        .iter()
        .find(|node| node.name == wanted)
        .map(|node| node.description.clone())
}

#[cfg(target_os = "linux")]
fn load_card_profiles() -> Vec<CardProfile> {
    let text = std::fs::read_to_string("/proc/asound/cards").unwrap_or_default();
    parse_asound_cards(&text)
        .into_iter()
        .map(|(index, id, _alsa_name)| {
            let usb = usb_identity_for_card(index);
            let model = usb.as_ref().and_then(usb_model_name);
            let distinguisher = usb.and_then(|info| info.serial.or(info.port));
            CardProfile {
                index,
                id,
                model,
                distinguisher,
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn parse_asound_cards(text: &str) -> Vec<(u32, String, String)> {
    text.lines().filter_map(parse_card_header).collect()
}

#[cfg(target_os = "linux")]
fn parse_card_header(line: &str) -> Option<(u32, String, String)> {
    let line = line.trim();
    let (index, rest) = line.split_once('[')?;
    let index = index.trim().parse().ok()?;
    let (id, rest) = rest.split_once(']')?;
    let name = rest
        .split_once(" - ")
        .map(|(_, name)| name.trim().to_string())
        .unwrap_or_default();
    Some((index, id.trim().to_string(), name))
}

#[cfg(target_os = "linux")]
fn usb_identity_for_card(index: u32) -> Option<UsbIdentity> {
    let mut path = std::fs::canonicalize(format!("/sys/class/sound/card{index}/device")).ok()?;
    for _ in 0..8 {
        if let Some(identity) = usb_identity_at(&path) {
            return Some(identity);
        }
        if !path.pop() {
            break;
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn usb_identity_at(path: &std::path::Path) -> Option<UsbIdentity> {
    let vendor = read_trimmed(&path.join("idVendor"))?;
    let product = read_trimmed(&path.join("idProduct"))?;
    if !is_usb_id(&vendor) || !is_usb_id(&product) {
        return None;
    }
    Some(UsbIdentity {
        vendor,
        product,
        product_string: read_trimmed(&path.join("product")),
        manufacturer: read_trimmed(&path.join("manufacturer")),
        serial: read_trimmed(&path.join("serial")).filter(|serial| useful_serial(serial)),
        port: path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
    })
}

#[cfg(target_os = "linux")]
fn is_usb_id(value: &str) -> bool {
    value.len() == 4 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(target_os = "linux")]
fn useful_serial(serial: &str) -> bool {
    let serial = serial.trim();
    !serial.is_empty() && serial.chars().any(|ch| ch != '0')
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(target_os = "linux")]
fn usb_model_name(info: &UsbIdentity) -> Option<String> {
    if let Some((vendor, model)) = query_hwdb(&info.vendor, &info.product) {
        let display = hwdb_display_name(&vendor, &model);
        if !display.is_empty() && !is_generic_usb_label(&display) {
            return Some(display);
        }
    }
    info.product_string.as_deref().and_then(|product| {
        if is_generic_usb_label(product) {
            None
        } else {
            Some(product_display_name(info.manufacturer.as_deref(), product))
        }
    })
}

#[cfg(target_os = "linux")]
fn query_hwdb(vendor: &str, product: &str) -> Option<(String, String)> {
    if !is_usb_id(vendor) || !is_usb_id(product) {
        return None;
    }
    let spec = format!(
        "usb:v{}p{}",
        vendor.to_ascii_uppercase(),
        product.to_ascii_uppercase()
    );
    let output = std::process::Command::new("systemd-hwdb")
        .arg("query")
        .arg(&spec)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut vendor_name = None;
    let mut model = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ID_VENDOR_FROM_DATABASE=") {
            vendor_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("ID_MODEL_FROM_DATABASE=") {
            model = Some(value.trim().to_string());
        }
    }
    let model = model.filter(|model| !model.is_empty())?;
    Some((vendor_name.unwrap_or_default(), model))
}

#[cfg(target_os = "linux")]
fn hwdb_display_name(vendor: &str, model: &str) -> String {
    let model = model.trim();
    if !is_generic_usb_label(model) {
        return model.to_string();
    }
    let vendor = strip_company_suffix(vendor);
    if vendor.is_empty() {
        model.to_string()
    } else {
        format!("{vendor} {model}")
    }
}

#[cfg(target_os = "linux")]
fn product_display_name(manufacturer: Option<&str>, product: &str) -> String {
    let product = product.trim();
    let Some(manufacturer) = manufacturer
        .map(strip_company_suffix)
        .filter(|name| !name.is_empty())
    else {
        return product.to_string();
    };
    if product
        .to_ascii_lowercase()
        .starts_with(&manufacturer.to_ascii_lowercase())
    {
        product.to_string()
    } else {
        format!("{manufacturer} {product}")
    }
}

#[cfg(target_os = "linux")]
fn strip_company_suffix(name: &str) -> &str {
    let mut name = name.trim();
    const SUFFIXES: &[&str] = &[
        " Co., Ltd.",
        ", Inc.",
        ", Inc",
        " Inc.",
        ", Ltd.",
        ", Ltd",
        " Ltd.",
        " Corporation",
        " Corp.",
        " Company",
    ];
    loop {
        let lower = name.to_ascii_lowercase();
        let next = SUFFIXES.iter().find_map(|suffix| {
            lower
                .strip_suffix(&suffix.to_ascii_lowercase())
                .map(|_| &name[..name.len() - suffix.len()])
        });
        match next {
            Some(stripped) if !stripped.trim().is_empty() && stripped.len() < name.len() => {
                name = stripped.trim();
            }
            _ => break,
        }
    }
    name
}

#[cfg(target_os = "linux")]
fn load_pipewire_catalog() -> PwCatalog {
    let output = match std::process::Command::new("pw-dump")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return PwCatalog::default(),
    };
    parse_pw_catalog(&output.stdout)
}

#[cfg(target_os = "linux")]
fn parse_pw_catalog(bytes: &[u8]) -> PwCatalog {
    let Ok(values) = serde_json::from_slice::<Vec<serde_json::Value>>(bytes) else {
        return PwCatalog::default();
    };
    let mut catalog = PwCatalog::default();
    for value in values {
        if let Some(entries) = value
            .get("metadata")
            .and_then(|metadata| metadata.as_array())
        {
            for entry in entries {
                let Some(key) = entry.get("key").and_then(|key| key.as_str()) else {
                    continue;
                };
                let Some(name) = entry.get("value").and_then(metadata_name) else {
                    continue;
                };
                match key {
                    "default.audio.source" => catalog.default_source_name = Some(name),
                    "default.audio.sink" => catalog.default_sink_name = Some(name),
                    _ => {}
                }
            }
        }
        let Some(props) = object_props(&value) else {
            continue;
        };
        let Some(class) = json_str(props, "media.class") else {
            continue;
        };
        let Some(card) = props.get("alsa.card").and_then(json_u32) else {
            continue;
        };
        if class == "Audio/Device"
            && let Some(description) = json_str(props, "device.description")
        {
            catalog.card_descriptions.insert(card, description);
            continue;
        }
        let capture = match class.as_str() {
            "Audio/Source" => true,
            "Audio/Sink" => false,
            _ => continue,
        };
        let Some(device) = props.get("alsa.device").and_then(json_u32) else {
            continue;
        };
        let Some(description) = json_str(props, "node.description") else {
            continue;
        };
        let Some(name) = json_str(props, "node.name") else {
            continue;
        };
        catalog.nodes.push(PwNodeLabel {
            name,
            card,
            device,
            capture,
            description,
        });
    }
    catalog
}

#[cfg(target_os = "linux")]
fn object_props(value: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    value
        .get("info")
        .and_then(|info| info.get("props"))
        .and_then(|props| props.as_object())
        .or_else(|| value.get("props").and_then(|props| props.as_object()))
}

#[cfg(target_os = "linux")]
fn json_str(props: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(target_os = "linux")]
fn json_u32(value: &serde_json::Value) -> Option<u32> {
    match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .and_then(|number| u32::try_from(number).ok()),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn metadata_name(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => map
            .get("name")
            .and_then(|name| name.as_str())
            .map(str::to_string),
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|parsed| {
                parsed
                    .get("name")
                    .and_then(|name| name.as_str())
                    .map(str::to_string)
            })
            .or_else(|| Some(text.clone())),
        _ => None,
    }
}

/// A live microphone capture handle.
///
/// While this struct is alive, the audio stream is active and RMS levels
/// are sent through the provided `flume::Sender<f32>`.
///
/// Dropping this struct stops the stream and releases audio resources.
pub struct MicCapture {
    _stream: cpal::Stream,
}

impl MicCapture {
    /// Start capturing audio from the device identified by `device_id`
    /// (matching the `id` field of `AudioDeviceInfo`).
    ///
    /// RMS levels are computed per-buffer and sent through `sender`.
    /// Returns an error if the device cannot be found or the stream fails to open.
    pub fn start(device_id: &str, sender: Sender<f32>) -> Result<Self, MicCaptureError> {
        let host = cpal::default_host();

        #[cfg(target_os = "windows")]
        let resolved_device_id = resolve_input_device_id(device_id)
            .map_err(|error| MicCaptureError::DeviceNotFound(error.to_string()))?;
        #[cfg(target_os = "windows")]
        let device_id = resolved_device_id.as_str();

        let mut devices = host
            .input_devices()
            .map_err(MicCaptureError::EnumerationFailed)?;
        let device = devices
            .find(|d| matches!(d.id(), Ok(id) if id.to_string() == device_id))
            .ok_or_else(|| MicCaptureError::DeviceNotFound(device_id.to_string()))?;

        let config =
            device
                .default_input_config()
                .map_err(|e: cpal::DefaultStreamConfigError| {
                    MicCaptureError::ConfigError(e.to_string())
                })?;

        let err_fn = move |err| {
            tracing::warn!("Audio stream error: {err}");
        };

        let sender = Mutex::new(sender);

        let stream = device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let rms = compute_rms(data);
                    if let Ok(s) = sender.lock() {
                        let _ = s.try_send(rms);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e: cpal::BuildStreamError| MicCaptureError::StreamError(e.to_string()))?;

        stream
            .play()
            .map_err(|e: cpal::PlayStreamError| MicCaptureError::StreamError(e.to_string()))?;

        Ok(Self { _stream: stream })
    }
}

/// Format of the PCM delivered by [`MicPcmCapture`].
#[derive(Debug, Clone, Copy)]
pub struct MicPcmFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// A live microphone capture handle that delivers raw interleaved PCM.
///
/// Unlike [`MicCapture`], which reduces every buffer to a single RMS level,
/// this hands the samples themselves to `sender` so they can be encoded.
///
/// Dropping this struct stops the stream and releases audio resources.
pub struct MicPcmCapture {
    _stream: cpal::Stream,
}

impl MicPcmCapture {
    /// Start capturing raw PCM from the default input device.
    ///
    /// Returns the device format alongside the handle; the samples are
    /// interleaved `f32` in that format, converted from whatever the device
    /// natively provides.
    ///
    /// `sender` must be unbounded: the samples are handed over from the
    /// high-priority audio thread with a non-blocking send, so a bounded
    /// channel that fills up would silently drop captured audio.
    pub fn start(sender: Sender<Vec<f32>>) -> Result<(Self, MicPcmFormat), MicCaptureError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| MicCaptureError::DeviceNotFound("default input".to_string()))?;

        let supported =
            device
                .default_input_config()
                .map_err(|e: cpal::DefaultStreamConfigError| {
                    MicCaptureError::ConfigError(e.to_string())
                })?;

        let format = MicPcmFormat {
            sample_rate: supported.sample_rate(),
            channels: supported.channels(),
        };
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let stream = match sample_format {
            cpal::SampleFormat::I8 => build_pcm_stream::<i8>(&device, &config, sender),
            cpal::SampleFormat::I16 => build_pcm_stream::<i16>(&device, &config, sender),
            cpal::SampleFormat::I24 => build_pcm_stream::<cpal::I24>(&device, &config, sender),
            cpal::SampleFormat::I32 => build_pcm_stream::<i32>(&device, &config, sender),
            cpal::SampleFormat::I64 => build_pcm_stream::<i64>(&device, &config, sender),
            cpal::SampleFormat::U8 => build_pcm_stream::<u8>(&device, &config, sender),
            cpal::SampleFormat::U16 => build_pcm_stream::<u16>(&device, &config, sender),
            cpal::SampleFormat::U32 => build_pcm_stream::<u32>(&device, &config, sender),
            cpal::SampleFormat::U64 => build_pcm_stream::<u64>(&device, &config, sender),
            cpal::SampleFormat::F32 => build_pcm_stream::<f32>(&device, &config, sender),
            cpal::SampleFormat::F64 => build_pcm_stream::<f64>(&device, &config, sender),
            other => {
                return Err(MicCaptureError::ConfigError(format!(
                    "unsupported input sample format {other:?}"
                )));
            }
        }
        .map_err(|e: cpal::BuildStreamError| MicCaptureError::StreamError(e.to_string()))?;

        stream
            .play()
            .map_err(|e: cpal::PlayStreamError| MicCaptureError::StreamError(e.to_string()))?;

        Ok((Self { _stream: stream }, format))
    }
}

fn build_pcm_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sender: Sender<Vec<f32>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let sender = Mutex::new(sender);
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let pcm = data
                .iter()
                .map(|sample| f32::from_sample(*sample))
                .collect::<Vec<f32>>();
            send_pcm(&sender, pcm);
        },
        |err| tracing::warn!("Mic capture stream error: {err}"),
        None,
    )
}

fn send_pcm(sender: &Mutex<Sender<Vec<f32>>>, pcm: Vec<f32>) {
    if let Ok(sender) = sender.lock() {
        let _ = sender.try_send(pcm);
    }
}

/// Compute the root-mean-square level of a buffer of f32 samples.
///
/// Returns `sqrt(sum(samples²) / samples.len())`.
/// For a buffer of all zeros, returns 0.0.
/// For a full-scale signal (±1.0), returns approximately 1.0.
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Errors that can occur when starting mic capture.
#[derive(Debug, thiserror::Error)]
pub enum MicCaptureError {
    #[error("Device enumeration failed: {0}")]
    EnumerationFailed(cpal::DevicesError),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Could not get device config: {0}")]
    ConfigError(String),

    #[error("Stream error: {0}")]
    StreamError(String),
}

#[cfg(all(test, target_os = "linux"))]
mod linux_device_names {
    use super::{
        CardProfile, PcmCandidate, PwCatalog, PwNodeLabel, assign_display_names, card_index_for,
        hwdb_display_name, is_generic_usb_label, is_placeholder_default_name, parse_asound_cards,
        parse_pw_catalog, pcm_card_token, pcm_dev_index, pipewire_default_name,
        product_display_name, select_pcm_candidates, strip_company_suffix,
    };

    fn card(index: u32, id: &str, model: Option<&str>, distinguisher: Option<&str>) -> CardProfile {
        CardProfile {
            index,
            id: id.to_string(),
            model: model.map(str::to_string),
            distinguisher: distinguisher.map(str::to_string),
        }
    }

    fn pcm(rank: u8, card_index: u32, dev: u32, id: &str, alsa_line: &str) -> PcmCandidate {
        PcmCandidate {
            rank,
            card: card_index,
            dev,
            id: id.to_string(),
            alsa_line: alsa_line.to_string(),
            digital: id.starts_with("hdmi:") || id.starts_with("iec958:"),
        }
    }

    #[test]
    fn parses_alsa_card_headers() {
        let text = "\
 0 [U0x46d0x825    ]: USB-Audio - USB Device 0x46d:0x825
                      USB Device 0x46d:0x825 at usb-0000:00:14.0-9.1, high speed
 1 [Headset        ]: USB-Audio - Logitech USB Headset
 2 [PCH            ]: HDA-Intel - HDA Intel PCH
";
        let cards = parse_asound_cards(text);
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0].0, 0);
        assert_eq!(cards[0].1, "U0x46d0x825");
        assert_eq!(cards[0].2, "USB Device 0x46d:0x825");
        assert_eq!(cards[1].1, "Headset");
        assert_eq!(cards[2].2, "HDA Intel PCH");
    }

    #[test]
    fn resolves_card_id_and_numeric_index_to_the_same_card() {
        let cards = vec![
            card(0, "U0x46d0x825", None, None),
            card(1, "Headset", None, None),
        ];
        assert_eq!(card_index_for("U0x46d0x825", &cards), 0);
        assert_eq!(card_index_for("0", &cards), 0);
        assert_eq!(card_index_for("Headset", &cards), 1);
        assert_eq!(pcm_card_token("sysdefault:CARD=Headset"), Some("Headset"));
        assert_eq!(pcm_dev_index("sysdefault:CARD=PCH"), 0);
        assert_eq!(pcm_dev_index("hw:CARD=PCH,DEV=2"), 2);
    }

    #[test]
    fn keeps_every_microphone_when_alsa_names_match() {
        let cards = vec![
            card(0, "CamA", Some("Webcam C270"), Some("235E6DE0")),
            card(1, "CamB", Some("Webcam C270"), Some("999ABC")),
        ];
        let selected = select_pcm_candidates(vec![
            pcm(
                0,
                0,
                0,
                "sysdefault:CARD=CamA",
                "USB Device 0x46d:0x825, USB Audio",
            ),
            pcm(
                3,
                0,
                0,
                "hw:CARD=CamA,DEV=0",
                "USB Device 0x46d:0x825, USB Audio",
            ),
            pcm(
                0,
                1,
                0,
                "sysdefault:CARD=CamB",
                "USB Device 0x46d:0x825, USB Audio",
            ),
        ]);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].id, "sysdefault:CARD=CamA");
        let named = assign_display_names(&selected, true, &cards, &PwCatalog::default());
        let mut names: Vec<_> = named.into_iter().map(|device| device.name).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "Webcam C270 (235E6DE0)".to_string(),
                "Webcam C270 (999ABC)".to_string()
            ]
        );
    }

    #[test]
    fn keeps_distinct_capture_endpoints_on_one_card() {
        let cards = vec![card(2, "PCH", None, None)];
        let pipewire = PwCatalog {
            card_descriptions: std::collections::HashMap::from([(2, "Built-in Audio".to_string())]),
            nodes: vec![PwNodeLabel {
                name: "alsa_input.pci.analog-stereo".to_string(),
                card: 2,
                device: 0,
                capture: true,
                description: "Built-in Audio Analog Stereo".to_string(),
            }],
            ..PwCatalog::default()
        };
        let selected = select_pcm_candidates(vec![
            pcm(
                0,
                2,
                0,
                "sysdefault:CARD=PCH",
                "HDA Intel PCH, ALC897 Analog",
            ),
            pcm(
                3,
                2,
                2,
                "hw:CARD=PCH,DEV=2",
                "HDA Intel PCH, ALC897 Alt Analog",
            ),
            pcm(
                4,
                2,
                0,
                "dsnoop:CARD=PCH,DEV=0",
                "HDA Intel PCH, ALC897 Analog",
            ),
        ]);
        assert_eq!(selected.len(), 2);
        let named = assign_display_names(&selected, true, &cards, &pipewire);
        let mut names: Vec<_> = named.into_iter().map(|device| device.name).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "Built-in Audio Analog Stereo".to_string(),
                "Built-in Audio — ALC897 Alt Analog".to_string(),
            ]
        );
    }

    #[test]
    fn prefers_pipewire_model_names_for_usb_mics() {
        let cards = vec![
            card(0, "U0x46d0x825", Some("Webcam C270"), Some("235E6DE0")),
            card(1, "Headset", Some("960 Headset"), None),
        ];
        let pipewire = PwCatalog {
            nodes: vec![
                PwNodeLabel {
                    name: "alsa_input.webcam".to_string(),
                    card: 0,
                    device: 0,
                    capture: true,
                    description: "Webcam C270 Mono".to_string(),
                },
                PwNodeLabel {
                    name: "alsa_input.headset".to_string(),
                    card: 1,
                    device: 0,
                    capture: true,
                    description: "960 Headset Mono".to_string(),
                },
            ],
            default_source_name: Some("alsa_input.headset".to_string()),
            ..PwCatalog::default()
        };
        let selected = select_pcm_candidates(vec![
            pcm(
                0,
                0,
                0,
                "sysdefault:CARD=U0x46d0x825",
                "USB Device 0x46d:0x825, USB Audio",
            ),
            pcm(
                0,
                1,
                0,
                "sysdefault:CARD=Headset",
                "Logitech USB Headset, USB Audio",
            ),
        ]);
        let named = assign_display_names(&selected, true, &cards, &pipewire);
        let mut names: Vec<_> = named.into_iter().map(|device| device.name).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "960 Headset Mono".to_string(),
                "Webcam C270 Mono".to_string()
            ]
        );
        assert_eq!(
            pipewire_default_name(&pipewire, true).as_deref(),
            Some("960 Headset Mono")
        );
    }

    #[test]
    fn drops_digital_aliases_when_an_analog_endpoint_exists() {
        let selected = select_pcm_candidates(vec![
            pcm(
                0,
                1,
                0,
                "sysdefault:CARD=Headset",
                "Logitech USB Headset, USB Audio",
            ),
            pcm(
                4,
                1,
                0,
                "iec958:CARD=Headset,DEV=0",
                "Logitech USB Headset, USB Audio",
            ),
        ]);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "sysdefault:CARD=Headset");
    }

    #[test]
    fn parses_pipewire_dump_labels_and_defaults() {
        let dump = r#"[
          {"info":{"props":{"media.class":"Audio/Device","device.description":"Webcam C270","alsa.card":0}}},
          {"info":{"props":{"media.class":"Audio/Source","node.name":"alsa_input.webcam","node.description":"Webcam C270 Mono","alsa.card":"0","alsa.device":0}}},
          {"metadata":[{"key":"default.audio.source","value":{"name":"alsa_input.webcam"}}]}
        ]"#;
        let catalog = parse_pw_catalog(dump.as_bytes());
        assert_eq!(
            catalog.card_descriptions.get(&0).map(String::as_str),
            Some("Webcam C270")
        );
        assert_eq!(catalog.nodes.len(), 1);
        assert_eq!(catalog.nodes[0].description, "Webcam C270 Mono");
        assert_eq!(
            catalog.default_source_name.as_deref(),
            Some("alsa_input.webcam")
        );
        assert!(
            is_generic_usb_label("USB Device 0x46d:0x825, USB Audio")
                || is_generic_usb_label("USB Audio")
        );
        assert!(is_generic_usb_label("USB Device 0x46d:0x825"));
        assert!(!is_generic_usb_label("Webcam C270"));
        assert!(is_placeholder_default_name("Default Audio Device"));
        assert_eq!(
            hwdb_display_name("Logitech, Inc.", "Webcam C270"),
            "Webcam C270"
        );
        assert_eq!(strip_company_suffix("Logitech, Inc."), "Logitech");
        assert_eq!(
            product_display_name(Some("Logitech"), "Logitech USB Headset"),
            "Logitech USB Headset"
        );
    }
}

#[cfg(test)]
mod host_device_labels {
    use super::{
        AudioDeviceInfo, disambiguate_duplicate_names, is_generic_device_label, os_device_label,
    };
    use cpal::{DeviceDescriptionBuilder, DeviceType, InterfaceType};

    #[test]
    fn windows_usb_mic_uses_friendly_model_not_via_usb() {
        let description = DeviceDescriptionBuilder::new("USB Audio Device")
            .device_type(DeviceType::Microphone)
            .interface_type(InterfaceType::Usb)
            .add_extended_line("Microphone (Logitech 960 Headset)")
            .build();
        let label = os_device_label(&description);
        assert_eq!(label, "Microphone (Logitech 960 Headset)");
        assert!(!label.contains("via USB"));
        assert!(!label.contains("USB Audio Device"));
    }

    #[test]
    fn windows_keeps_specific_name_when_friendly_line_is_generic() {
        let description = DeviceDescriptionBuilder::new("Logitech Webcam C270")
            .interface_type(InterfaceType::Usb)
            .add_extended_line("Microphone")
            .build();
        assert_eq!(os_device_label(&description), "Logitech Webcam C270");
    }

    #[test]
    fn macos_uses_coreaudio_name_without_a_via_suffix() {
        let description = DeviceDescriptionBuilder::new("MacBook Pro Microphone").build();
        let label = os_device_label(&description);
        assert_eq!(label, "MacBook Pro Microphone");
        assert!(!label.contains("via "));
    }

    #[test]
    fn identical_usb_mics_stay_listed_and_numbered() {
        let mut devices = vec![
            AudioDeviceInfo {
                id: "a".to_string(),
                name: "Microphone (USB Audio Device)".to_string(),
            },
            AudioDeviceInfo {
                id: "b".to_string(),
                name: "Microphone (USB Audio Device)".to_string(),
            },
            AudioDeviceInfo {
                id: "c".to_string(),
                name: "Microphone (Logitech 960 Headset)".to_string(),
            },
        ];
        disambiguate_duplicate_names(&mut devices);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].name, "Microphone (USB Audio Device) (1)");
        assert_eq!(devices[1].name, "Microphone (USB Audio Device) (2)");
        assert_eq!(devices[2].name, "Microphone (Logitech 960 Headset)");
        assert_ne!(devices[0].id, devices[1].id);
        assert!(is_generic_device_label("USB Audio Device"));
        assert!(!is_generic_device_label(
            "Microphone (Logitech 960 Headset)"
        ));
    }
}
