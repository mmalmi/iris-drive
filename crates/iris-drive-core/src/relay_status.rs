pub fn relay_status_label(status: &str) -> String {
    if status == "configured" {
        "saved".to_owned()
    } else {
        status.to_owned()
    }
}

pub fn relay_status_health(status: &str) -> &'static str {
    match status {
        "connected" => "online",
        "connecting" => "connecting",
        "blocked" | "offline" | "terminated" => "error",
        "configured" => "configured",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_statuses_have_rust_owned_labels_and_health() {
        assert_eq!(relay_status_label("configured"), "saved");
        assert_eq!(relay_status_label("connected"), "connected");
        assert_eq!(relay_status_health("connected"), "online");
        assert_eq!(relay_status_health("connecting"), "connecting");
        assert_eq!(relay_status_health("blocked"), "error");
        assert_eq!(relay_status_health("offline"), "error");
        assert_eq!(relay_status_health("terminated"), "error");
        assert_eq!(relay_status_health("configured"), "configured");
        assert_eq!(relay_status_health("mystery"), "unknown");
    }
}
