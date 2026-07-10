/// Shared public FIPS bootstrap/transit nodes. Kept in sync with nostr-vpn's
/// defaults so native Iris instances can join the same relay overlay when
/// direct device-to-device UDP/WebRTC is unavailable.
pub(super) const DEFAULT_FIPS_BOOTSTRAP_PEERS: &[(&str, &[&str])] = &[
    (
        "npub1260n42s06vzc7796w0fh3ny7zcpw6tlk4gq3940gmfrzl5c9pv2s3657q8",
        &["udp:217.160.76.169:2121"],
    ),
    (
        "npub17lpmzulpc98d8ff727k6e98atxn3phzupzsqqwe54ytduym747ws4tw5zm",
        &["udp:82.223.139.182:2121"],
    ),
    (
        "npub1u0z26dc4qeneu5rvwvmpfhtwh3522ed6rlgxr9jarrfnjrc6ew4qxjysrs",
        &["udp:88.208.241.33:2121"],
    ),
    (
        "npub1qmc3cvfz0yu2hx96nq3gp55zdan2qclealn7xshgr448d3nh6lks7zel98",
        &["udp:217.77.8.91:2121", "tcp:217.77.8.91:443"],
    ),
    (
        "npub10yffd020a4ag8zcy75f9pruq3rnghvvhd5hphl9s62zgp35s560qrksp9u",
        &["udp:23.182.128.74:2121", "tcp:23.182.128.74:443"],
    ),
    (
        "npub136yqae6na688fs75g95ppps3lxe07fvxefj77938zf47uhm6074sxw8ctm",
        &["udp:54.183.70.180:2121", "tcp:54.183.70.180:443"],
    ),
    (
        "npub1gd7ye2qp2lphhzx75fynnjzaxx4dqanddecet0wtt5ss5ek8h9ps62wdkf",
        &["udp:74.208.245.160:2121"],
    ),
];
