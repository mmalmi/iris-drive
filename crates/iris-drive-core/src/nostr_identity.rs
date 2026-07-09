//! Canonical NostrIdentity protocol surface.
//!
//! Iris Drive keeps its local Drive content key (DCK) model in app-level
//! projections, but the shared NostrIdentity wire protocol lives in
//! `nostr_identity`.

pub use nostr_identity::{
    KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE, KIND_NOSTR_IDENTITY_ROSTER_OP,
    NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA, NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE,
    NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT, NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA,
    NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA, NOSTR_IDENTITY_ROSTER_SCHEMA,
    NostrIdentityCapabilities, NostrIdentityDeviceApprovalReceipt,
    NostrIdentityDeviceApprovalRequest, NostrIdentityEncryptedDeviceLabelsPayload,
    NostrIdentityError, NostrIdentityFacet, NostrIdentityFacetAcceptanceContent, NostrIdentityId,
    NostrIdentityKeyPurpose, NostrIdentityRosterLog, NostrIdentityRosterOp,
    NostrIdentityRosterOpContent, NostrIdentityRosterProjection, NostrIdentitySecretEpoch,
    NostrIdentityTombstone, SecretWrapStatus, SignedNostrIdentityFacetAcceptance,
    SignedNostrIdentityRosterOp, build_nostr_identity_device_approval_receipt_event,
    build_nostr_identity_facet_acceptance_event, build_nostr_identity_roster_op_event,
    build_nostr_identity_roster_op_event_with_encrypted_device_labels,
    encrypted_device_label_payloads_from_nostr_identity_roster_op_event,
    encrypted_profile_payloads_from_nostr_identity_roster_op_event,
    is_nostr_identity_facet_acceptance_event_coordinate,
    is_nostr_identity_roster_op_event_coordinate,
    nostr_identity_candidate_ids_for_pubkey_from_events, nostr_identity_facet_acceptance_d_tag,
    nostr_identity_ids_from_facet_acceptances, nostr_identity_roster_op_d_tag,
    nostr_identity_roster_parent_ids, nostr_identity_tag_kind,
    parse_nostr_identity_device_approval_receipt_event,
    parse_nostr_identity_device_approval_receipt_roster_op,
    parse_nostr_identity_facet_acceptance_event, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster, validate_signed_nostr_identity_roster_op,
};
