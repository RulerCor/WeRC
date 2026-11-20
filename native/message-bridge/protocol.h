#pragma once
#include <cstdint>
inline constexpr uint32_t WercProtocolVersion = 27;
// ABI24: MomentInteract action3 cancels own like; action4 deletes own post.
// All are exact post IDs. Native author verification precedes own-post deletion.
// Action5 replies to a genuine comment from fresh native detail: present=id32,
// sequence=id64 (at least one nonzero), text[0..896]=UTF8 body (max895 bytes),
// text[896..1024]=exact comment author. IDs and author must match one native item.
// Binary ABI, local memory only. No pointers, paths, arbitrary function/SQL
// requests. Prepared destinations bind all sends; Poll carries native receive routes.
enum class WercOp : uint32_t { Inspect=1, Attach=2, Send=3, Poll=4, Receipt=5, Close=6, Ack=7, Recover=8, VoiceBegin=9, VoiceChunk=10, VoiceProbe=11, VoiceSend=12, ImageBegin=13, ImageChunk=14, ImageProbe=15, ImageSend=16, FileBegin=17, FileChunk=18, FileProbe=19, FileSend=20, ReceiveChunk=21, DownloadImageBegin=22, DownloadPoll=23, DownloadChunk=24, DownloadRelease=25, ReceiveVoiceChunk=26, DownloadFileBegin=27, DownloadVideoBegin=28, VideoBegin=29, VideoChunk=30, VideoProbe=31, VideoSend=32, EmoticonBegin=33, EmoticonChunk=34, EmoticonProbe=35, EmoticonSend=36, PrepareTarget=37, MentionStage=38, MentionSend=39, MentionProbe=40, TextBegin=41, TextChunk=42, TextSend=43, TextProbe=44, QuoteStage=45, QuoteProbe=46, QuoteSend=47, QuoteChunk=48, DownloadOriginalImageBegin=49, MomentBegin=50, MomentChunk=51, MomentProbe=52, MomentSend=53, MomentImageBegin=54, MomentImageChunk=55, MomentImageCommit=56, DownloadFavoriteBegin=57, MomentInteract=58, MomentLookup=59, MomentVideoBegin=60, MomentVideoChunk=61, MomentVideoCommit=62, ArticleBegin=63, ArticleInput=64, ArticleRead=65, ArticlePoll=66, ArticleChunk=67, ArticleRelease=68, ArticleOpen=69, SettingsRead=70, SettingsApply=71 };
// ABI v2: Poll retains one in-flight message until Ack(sequence). An older
// destructive consumer must fail the version check, never silently lose data.
// ABI v3 retains the packet size and adds bounded staged SILK upload. Begin:
// message_id=total bytes, dropped=duration_ms, text=SHA256; Chunk:
// sequence=byte offset, present=chunk bytes. No upload itself sends a message.
struct WercPacket {
    uint32_t size, version, operation, status;
    uint32_t pid, controller_pid;
    uint64_t process_start, controller_start, deadline;
    unsigned char nonce[32];
    char account[128], target[128], attempt[64], digest[65], text[1024];
    uint32_t receipt, present, dropped, reserved;
    uint64_t sequence, message_id;
};
static_assert(sizeof(WercPacket) == 1528, "Protocol ABI changed");
// receipt 0 unknown; 1 queued; 2 native callback success; 3 callback failure;
// 4 rejected before submission. Values 1/2/3 do NOT prove recipient delivery.
// 5 voice native submit returned zero; 6 voice object probe only (no send).

// ABI v4 keeps size and adds canonical PNG staging. Image Begin: message_id
// is byte length; dropped packs width | height<<16; text is SHA256. Images
// use distinct attempt kinds. receipt 7 is native return zero, 8 is probe only.

// ABI v5: file Begin text[0..65] SHA256, text+65 UTF-8 basename (<=120 bytes),
// message_id=byte length (1..8MiB), dropped=0. receipt 9 native zero / 10 probe.

// ABI v6: Poll receipt=message type, deadline=content byte length (<=16383).
// ReceiveChunk sequence=retained observation, dropped=byte offset, response
// present=chunk bytes, text=binary UTF-8 slice. ACK only after durable commit.
// The packet size stays fixed. Media content is metadata, never downloaded bytes.

// ABI v7: medium image download. Begin text[0..769]=hex filekey,
// text[769..802]=AES hex; message_id=declared size <=8MiB, dropped=2.
// Poll receipt=1 pending/2 ready/3 failed, dropped=Windows error on failure.
// Ready message_id is actual bytes <=8MiB, independent of XML declared size.
// These are declaration/completion checks, not an in-flight disk hard limit.
// Chunk sequence=offset returns present<=1024 bytes. Release cancels/clears.
// All calls bind the same attempt/digest and authenticated connection.

// ABI v8: Poll response reserved=inline voice bytes (0..262144), only type34.
// Requests still require reserved=0. ReceiveVoiceChunk uses sequence + dropped
// byte offset; response present is byte count, deadline is total voice bytes.
// Bytes are retained with the observation until durable commit followed by ACK.

// ABI v9: DownloadFileBegin uses the bounded v7 descriptor layout, dropped=5.
// Ordinary appmsg/type6 only; completed bytes must match declared size exactly.

// ABI v10: DownloadVideoBegin uses the v7 descriptor layout, dropped=4.
// Ordinary type43 only; declared and completed sizes are separately <=16MiB.
// Native media transformation can change length. Consumers must validate content.

// ABI v11: video staging 29..32, file layout plus dropped=duration_ms (1..60s).
// MP4 <=8MiB. Native duration is queried from owned staged bytes.
// receipt 11 native zero / 12 probe; Receipt response dropped=native seconds.

// ABI v12: GIF staging 33..36, file layout with packed width|height<<16.
// <=1MiB/1024px, distinct attempt kinds 9/10; receipt 13 native zero / 14 probe.

// ABI v13: Poll target=conversation, text[0..128]=sender, text[128..256]=receiver.
// Both IDs must be bounded ASCII and one endpoint must match authenticated login.
// Group envelopes identify the group, not a verified member author. Body uses chunks.

// ABI v14: PrepareTarget registers a bounded exact target until deadline (<=300s).
// Staging and receipts bind target. Preparation never schedules a native send.

// ABI15: 38 stages 1..8 comma-separated member IDs in text; 39 submits text
// with the same target/attempt/digest/deadline. 40 constructs only, receipt15.
// Staging never sends. Only group targets; packet size remains unchanged.

// ABI16: TextBegin text=SHA256, message_id=1024..10000 UTF-8 bytes, dropped=0
// plain / 1 staged mentions. TextChunk sequence=offset, present=1..1024 bytes.
// TextSend and TextProbe require complete bytes/hash; probe receipt16, no send.
// Target/attempt/request digest/deadline/size/member mode remain immutable.

// ABI18: operation49 selects original image type1; both declared/completed bytes
// are bounded to 8MiB and must match. The caller also verifies original MD5.

// ABI19: private text moment (visibility dropped=2), total UTF-8 bytes 1..10000
// in message_id. Begin text=body SHA256; Chunk sequence/ present=offset/count.
// Probe only constructs and connects/cancels the completion event; receipt20.
// Send requires exact authenticated self target, receipt kind21. Receipt/finish
// response present=kind, message_id=SNS ID only after matching completion.
// receipt21 proves server acceptance, never recipient/phone confirmation.

// Receipt optional sequence: candidate outgoing plain-text server ID observed
// on the retained factory object after its exact target/body callback. Zero
// means unavailable. This never changes receipt evidence or proves delivery.
// message_id keeps its existing quote-source / SNS meaning.

// ABI v21: DownloadFavoriteBegin uses the bounded descriptor, with native
// media type 2/4/5 and the Favorites CDN scene. Chat downloads keep scene 1.

// ABI v22: MomentInteract(58) and MomentLookup(59) bind an exact nonzero
// SNS message_id to the authenticated self target. dropped=1 like / 2 comment
// (lookup requires 0); text carries the UTF-8 comment, at most 1023 bytes.
// Receipt kind22 is lookup, kind23 interaction. Evidence22 means local model
// found; native bool/queued never proves server acceptance or recipient delivery.
// Repeated attempts only query the retained receipt; they never submit again.

// ABI23: MomentBegin receipt=1 selects a video moment (present must be one PNG
// cover). Ops60/61/62 stage one MP4; message_id=length, dropped=duration_ms,
// Begin/Commit sequence=width|height<<16, text=SHA256. Chunk sequence=offset.
// Same request binding throughout; complete cover+video required before probe/send.

// ABI25: Fixed publisher-history read (not arbitrary JavaScript/native RPC).
// 63: target=gh_ publisher, sequence=unique uint32 session, message_id=offset bytes
// (0..8192), digest=SHA256(offset), deadline<=45s. 64: sequence=byte offset,
// present=1..1024. 65 executes once; duplicates fail, never submit twice.
// 66: receipt=1 staged/2 queued/3 matched/4 complete, dropped=error,
// message_id=response bytes<=2MiB. 67: sequence=offset, present=returned bytes.
// 68 releases only this exact attempt. All operations bind login/controller.

// ABI26: 69 stages a canonical public article URL in 64 input chunks. 65 issues
// one client navigation; stage4 means issued only. Body proof is read separately.

// ABI27: fixed background native settings. 70 reads mask present=7 and values
// dropped bits 0 sound, 1 call sound, 2 auto-download. 71: dropped=id 1..3,
// present=boolean target, sequence=expected boolean or 2 (unspecified).
// Response receipt=1 verified, 2 unchanged, 3 conflict, 4 readback mismatch;
// message_id=before, sequence=observed. Both bind current login and deadline.
