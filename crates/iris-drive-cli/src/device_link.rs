#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_devices(config_dir: &std::path::Path, command: DevicesCmd) -> Result<()> {
    match command {
        DevicesCmd::Invite => cmd_devices_invite(config_dir),
        DevicesCmd::Request {
            owner_or_invite,
            admin_device,
            label,
        } => {
            cmd_link_with_admin_device(config_dir, &owner_or_invite, admin_device.as_deref(), label)
        }
        DevicesCmd::Requests => cmd_devices_requests(config_dir),
        DevicesCmd::Approve { request, label } => cmd_approve(config_dir, &request, label),
        DevicesCmd::List => cmd_roster(config_dir),
        DevicesCmd::Revoke { device } => cmd_revoke(config_dir, &device),
        DevicesCmd::AppointAdmin { device } => cmd_appoint_admin(config_dir, &device),
        DevicesCmd::DemoteAdmin { device } => cmd_demote_admin(config_dir, &device),
    }
}

pub(crate) fn cmd_devices_invite(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let invite = device_link_invite_json(state);
    if invite.is_null() {
        return Err(anyhow::anyhow!(
            "device link invites require an admin device"
        ));
    }
    println!("{invite}");
    Ok(())
}

pub(crate) fn cmd_devices_requests(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    println!(
        "{}",
        json!({
            "outbound": device_link_request_json(state),
            "inbound": inbound_device_link_requests_json(state),
        })
    );
    Ok(())
}

pub(crate) fn percent_encode_component(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

pub(crate) fn percent_decode_component(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = bytes
                .get(index + 1)
                .copied()
                .and_then(hex_value)
                .ok_or_else(|| anyhow::anyhow!("invalid percent encoding"))?;
            let lo = bytes
                .get(index + 2)
                .copied()
                .and_then(hex_value)
                .ok_or_else(|| anyhow::anyhow!("invalid percent encoding"))?;
            output.push((hi << 4) | lo);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(output).context("request contains invalid UTF-8")
}

pub(crate) fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => '0',
    }
}

pub(crate) fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
