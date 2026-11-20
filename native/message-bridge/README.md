# WeRC native filehelper adapter (experimental)

This is a source-built, non-UIA Windows x64 adapter, derived from the
user-supplied helper ZIP recorded in `../helper/SOURCE.json`. The user confirmed
personal noncommercial use on 2026-11-11. The helper noncommercial license and
MinHook notices are retained in `../helper/`; this component cannot be shipped
as a commercially licensed implementation without separate rights.

`common.inc` and `core_version.h` are modified helper files. Changes: upstream
self-test exports made internal, disk reports disabled, native IDs retained in
the filehelper text receive queue, one exact local patched build added. The
new public bridge API, inherited-pipe controller and Rust provider are WeRC
integration code. No prebuilt upstream executable or DLL was imported.

Build from the project root using `scripts/build-message-bridge.ps1`. A new
output directory contains the bridge, a controller compiled against that
bridge's SHA-256, an owned fixture executable, and build evidence. The script
verifies pristine upstream inputs against `SOURCE.json` and uses installed
MSVC executables from an independent build directory. No upstream Python or
build scripts are executed. Builds use the static MSVC runtime. Source hashes
are recorded; byte-for-byte binary reproducibility is not yet established.

The controller exposes only an inherited stdin/stdout fixed binary protocol.
There is no network listener or discoverable IPC server. A same-user/session
process check, per-user ACL on the single-owner mutex, PID creation identity,
component hash locks and client entry-byte checks precede loading. The Rust
parent assigns only its controller to a Job Object (128 MiB, one process,
20% CPU hard cap, kill on job close); the Weixin process is never assigned to
that job. The controller cannot accept arbitrary addresses, SQL or DLL paths.

Only the exact original 4.1.15.13 hash and the user's exact three-byte patched
variant are accepted. The patched binary remains modified and does not have
a valid publisher signature. Its unmodified local backup matches the upstream
hash and has a valid signature. Full file comparison found two changed
functions; the complete native send builder, receive handler and scheduler
submit functions are unchanged. This is a bounded compatibility candidate,
not a promise that arbitrary patches or all behavior are compatible. No recall
hook is enabled, and this adapter does not change the installed client files.

`NativeFileHelperProvider` implements the existing `CapabilityProvider`
resolve/submit/reconcile ports and uses existing outbox approval and durable
attempt handling. `ManagedWindowsWechatProvider` adds explicit desktop
connect/disconnect; startup does not attach. Only `filehelper` and <=1023
UTF-8 bytes are accepted. An account-scoped keyed identity token prevents a
different login from being silently attached to an existing native identity.
Queued tasks recheck identity and expiry. An attempt is never submitted twice
within a controller session; its receipt storage has a hard 128-attempt bound.
The durable product outbox owns restart recovery and never resubmits Unknown.
Native callback success is reported as `NativeQueued`, not server acceptance
or recipient delivery. There is no reliable server reconciliation after the
controller exits; manual recipient evidence must remain a separate record.

Receive has 64 queued events plus one retained in-flight event. ABI v9 Poll
repeats that event until Ack(sequence); overflow does not evict the in-flight
event. Oversized/unreadable text produces a gap instead of silent truncation.
Application commits AES-GCM/DPAPI-protected messages and a keyed checkpoint
in one SQLite transaction before acknowledging. Native IDs deduplicate across
sessions, conflicting content is rejected, and every attachment reports an
unproven interval. Quotas do not silently evict records. Desktop and the
authorized `/messages/native` API browse these observations separately from
imported history. This is not a DB commit feed, durable native replay stream,
full chat history, or proof that G1a/G1b online DB/WAL requirements passed.

Normal EOF removes hooks, waits for callbacks/tasks, clears buffers and
unloads. Controller death disables hooks via an in-process watcher, but leaves
an inert module mapped if safe unloading cannot be proven. Recovery requires
the same pinned image path, dead owner, completed watcher and zero native
tasks/callbacks. A recovered image stays resident even after a subsequent
clean disconnect, because an old timed-out remote call cannot be excluded.
Different builds are refused while that image remains; no client restart is
forced. The owned fixture covers these conditions; real-client crash recovery
has not been tested. This is not an OS
sandbox for the injected bridge and is not a completed worker safety audit.

The explicit `native_filehelper_smoke` example uses a new isolated product
state directory and the existing encrypted outbox. `connect` does not send.
`send-once` performs one authorized test submission and does not retry Unknown.
It is not run by cargo test or normal startup. Real-client runs must respect
the user's precise target/content authorization, preserve result uncertainty,
and separately verify receiver-side evidence. No normal user database is
opened or migrated by this example.

ABI v3 adds bounded SILK upload, a non-sending native type-34 probe, and a
single voice submission. It retains the 1528-byte frame and all identity,
digest, target, deadline and attempt checks. The separate source-built codec
accepts only 16 kHz mono PCM16, whole 20 ms frames, at most 60 seconds.
Native voice return zero means locally queued, never recipient delivery.
The voice example has submitted the one explicitly authorized test on
2026-11-11; do not run it again to refresh build evidence. Receiver evidence
and the normal provider's capability state are tracked separately in
`../../docs/verification/2026-11-11-native-voice.md`.

Image ABI v4 operations 13–16 accept bounded canonical PNG bytes, not caller
paths. The Application decodes and rewrites RGBA8 PNG, authenticates source
and canonical digests, and persists schema3 approvals in the existing outbox.
The bridge uses private pinned source staging. Both prepared and original
native object paths refer to Weixin's own InputTemp copy before dispatch;
that copy is left to Weixin after invocation, while our source is removed.
The probe checks the native constructor and path replacement without sending.
A zero native return is NativeQueued/Unknown, never automatic delivery.
The one authorized image and phone reopen confirmation are documented in
`../../docs/verification/2026-11-11-native-image.md`. Other image formats and
other image receive formats remain outside this bounded implementation; ABI v7 below adds medium-image receive.

File ABI v5 operations 17–20 stage nonempty bytes (at most 8 MiB) and a
validated UTF-8 basename. File names and contents are encrypted together in
product state. A type-49/subtype-6 native probe and one authorized file submission passed;
the user confirmed its contents and phone-reopen persistence. See the dated native-file verification record.

Receive ABI v6 keeps the 1528-byte packet. Poll returns type and byte length;
operation 21 reads 1024-byte slices from the retained event, bound to its
sequence and native ID. Only after full UTF-8 validation and authenticated
product commit does the consumer ACK. Content is limited to 16383 UTF-8
bytes per event. Types 3/34/43/47/49/62/10000/10002 retain raw metadata with
an empty text field; they never represent downloaded media or playable audio.
The desktop displays a type placeholder and never renders the raw XML. The
same messages:read permission protects API metadata. ABI v8 below adds a
separate bounded binary path; metadata-only observations remain readable.
Unsupported filehelper types and oversized/malformed metadata increment gaps.
Legacy schema1 text JSON and its content authentication remain unchanged.


## ABI v7 received image content

Operations 22–25 begin/poll/read/release a medium-image download for the bound
filehelper observation. Credentials are copied into the fixed local packet and
cleared after task construction; paths are generated internally. Only exact
profile 1 is enabled. The factory hook changes only the owned task pointer with
an unpredictable bridge prefix. No send constructor is used.

Declared and completed sizes are independently limited to 8MiB; this is not an
in-flight disk hard limit. Original bytes never enter the product image cache:
Rust fully decodes JPEG/static PNG with pixel/output budgets, strips metadata and
at most 64 bytes after the JPEG EOI, then encrypts canonical PNG. Existing receive
ACK can run between short download poll/chunk exchanges. Disconnect signals
cancel, releases the slot, and waits for active calls before host teardown;
native tasks retain their lease until completion (never terminate WeChat).


Receive ABI v8 adds operation26 and Poll.reserved as the inline voice length.
It captures at most262144 bytes from the exact profile1 AddMsg+0x30 buffer,
whose layout was independently checked in this build's constructor. It does
not copy offsets from newer profiles. Queue/inflight own the bytes until ACK,
with about18MiB fixed storage. Metadata-only observations remain valid when no
bounded binary is available. v17 stores the observation, encrypted asset,
HMAC association and cursor in one transaction before ACK. Decoding is explicit,
through the source-built pinned codec process; desktop/API return WAV only
after successful complete decoding, and never auto-play. The v8 real-device
sample and release status are tracked in native-voice-receive verification;
implementation and owned tests alone do not prove phone interoperability.

ABI v9 additionally exposes bounded ordinary attachment CDN downloads (operation
27, task type 5). Received-file XML must be appmsg subtype 6; native metadata is
not displayed as HTML. Schema 18 links encrypted file assets transactionally to
received observations. Current-device live-window attachment download has been
verified separately; a new inbound attachment hook event has not been verified.
Voice inline capture is implemented but still awaits an observed real payload.
Component paths are resolved relative to the controller executable at runtime.
Moving the complete component directory does not require rebuilding.

## ABI18 original images

Operation49 uses the existing bounded download request with type1 and exact completed size (at most 8MiB). Operation22 remains medium-image/type2. The Application derives credentials only from an authenticated forwarded record, checks full MD5 and decodes the complete original before encryption. Other record media types are not implied. Current exact patched 4.1.15.13 was verified without restarting or replacing it.
