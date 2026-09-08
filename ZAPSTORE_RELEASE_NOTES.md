# Iris Drive 0.1.35

- Update embedded Hashtree components and networking dependencies with security
  fixes.
- Complete slow peer downloads when backup storage has no copy, while preserving
  the last complete file view until every required block is available.
- Reduce repeated background retries while a mobile device is offline, while
  keeping pending device approvals responsive.
- Show Android device approval prompts immediately when a request arrives during
  native startup, while still requiring explicit confirmation.
- Recover missed desktop updates after a peer reconnects, while preserving the
  last complete file view until the newer signed root is available.
- Enforce mesh hop limits when forwarding missing blocks between peers,
  preventing repeated requests from circulating without consuming their budget.
