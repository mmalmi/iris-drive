# TCP/FIPS Hashtree blob v1

Iris Drive transfers Hashtree blobs on TCP/FIPS service `39018`. TCP owns
ordered byte delivery, flow control, and segment retransmission. Hashtree keeps
one bounded whole-session retry because an entirely reset FIPS session is
outside TCP's recoverable connection state.

The 35-byte request is magic `0x48` (`H`), version `0x01`, operation `0x01`
(get), and a 32-byte SHA-256 hash. The response begins with magic, version,
status `0x00` (missing) or `0x01` (found), and a big-endian unsigned 32-bit
payload length. A found response continues with exactly that many bytes.
Implementations reject blobs above 16 MiB and verify SHA-256 before caching or
returning them.

Shared Rust/TypeScript vectors for hash bytes `00..1f`:

- request: `480101000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f`
- found response header for three bytes: `48010100000003`
