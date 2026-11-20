//! Text/image publishing uses a separate protocol and completion rule from chat.
use super::*;

fn moment_receipt(
    id: &str,
    attempt: &str,
    digest: &str,
    result: &Packet,
    probe: bool,
) -> AppResult<SendReceipt> {
    let kind = if probe { 20 } else { 21 };
    if result.u32(1496) != kind {
        return Err(error("NATIVE_MOMENT_RECEIPT_MISMATCH"));
    }
    let code = result.u32(1492);
    let sns_id = result.u64(1520);
    let evidence = match code {
        21 if !probe && sns_id != 0 => SendEvidence::ServerAccepted,
        4 => SendEvidence::RejectedBeforeSubmission,
        1 => SendEvidence::NativeQueued,
        _ => SendEvidence::Unknown,
    };
    Ok(SendReceipt {
        outbox_id: id.into(),
        attempt_id: attempt.into(),
        context_digest: digest.into(),
        evidence,
        provider_reference: (evidence == SendEvidence::ServerAccepted)
            .then(|| format!("native-moment:{sns_id}")),
    })
}

impl NativeFileHelperProvider {
    fn dispatch_private_moment(
        &self,
        s: &TextSubmission<'_>,
        images: &[MomentImage<'_>],
        probe: bool,
        visibility: MomentVisibility,
    ) -> AppResult<Packet> {
        self.dispatch_moment_content(s, images, None, probe, visibility)
    }
    fn dispatch_moment_content(&self, s: &TextSubmission<'_>, images: &[MomentImage<'_>],
        video: Option<(&VideoSendAsset, &[u8])>, probe: bool, visibility: MomentVisibility) -> AppResult<Packet> {
        if let Some((asset, bytes)) = video {
            if images.len() != 1 || bytes.len() != asset.file.size
                || hex::encode(Sha256::digest(bytes)) != asset.file.sha256
                || werc_domain::video::inspect_send_mp4(&asset.file.file_name, bytes)? != asset.info {
                return Err(error("NATIVE_VIDEO_INVALID"));
            }
        }
        if !valid_moment_text(s.text, images.len()) || s.context.target_id != s.context.sender_id {
            return Err(error("NATIVE_MOMENT_INVALID"));
        }
        for image in images {
            let (info, canonical) = werc_domain::image::canonical_png(image.png)?;
            if canonical.as_slice() != image.png
                || info.width != image.asset.width
                || info.height != image.asset.height
                || hex::encode(Sha256::digest(image.png)) != image.asset.png_sha256
            {
                return Err(error("NATIVE_IMAGE_INVALID"));
            }
        }
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.sender_id)? {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut begin = channel_binding(&channel, 50);
        begin.text(TARGET, 128, &s.context.sender_id)?;
        begin.text(ATTEMPT, 64, s.attempt_id)?;
        begin.text(DIGEST, 65, s.request_digest)?;
        begin.text(TEXT, 65, &hex::encode(Sha256::digest(s.text.as_bytes())))?;
        begin.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        begin.put64(1520, s.text.len() as u64);
        begin.put32(
            1500,
            match visibility {
                MomentVisibility::Private => 2,
                MomentVisibility::Public => 1,
            },
        );
        begin.put32(1496, images.len() as u32);
        begin.put32(1492, u32::from(video.is_some()));
        let mut finish = begin.copy(if probe { 52 } else { 53 });
        finish.put32(1496, 0);
        channel.exchange(begin)?;
        for (i, bytes) in s.text.as_bytes().chunks(1024).enumerate() {
            let mut chunk = finish.copy(51);
            chunk.put64(1512, (i * 1024) as u64);
            chunk.put32(1496, bytes.len() as u32);
            chunk.0[TEXT..TEXT + 1024].fill(0);
            chunk.0[TEXT..TEXT + bytes.len()].copy_from_slice(bytes);
            channel.exchange(chunk)?;
        }
        for (index, image) in images.iter().enumerate() {
            let mut header = finish.copy(54);
            header.put64(1520, image.png.len() as u64);
            header.put64(1512, index as u64);
            header.put32(1500, image.asset.width | (image.asset.height << 16));
            header.text(TEXT, 1024, &image.asset.png_sha256)?;
            channel.exchange(header.copy(54))?;
            for (index, bytes) in image.png.chunks(1024).enumerate() {
                let mut chunk = header.copy(55);
                chunk.put64(1512, (index * 1024) as u64);
                chunk.put32(1496, bytes.len() as u32);
                chunk.0[TEXT..TEXT + 1024].fill(0);
                chunk.0[TEXT..TEXT + bytes.len()].copy_from_slice(bytes);
                channel.exchange(chunk)?;
            }
            channel.exchange(header.copy(56))?;
        }
        if let Some((asset, bytes)) = video {
            let mut header = finish.copy(60);
            header.put64(1520, bytes.len() as u64);
            header.put64(1512, u64::from(asset.info.width) | (u64::from(asset.info.height) << 16));
            header.put32(1500, asset.info.duration_ms as u32);
            header.text(TEXT, 1024, &asset.file.sha256)?;
            channel.exchange(header.copy(60))?;
            for (index, bytes) in bytes.chunks(1024).enumerate() {
                let mut chunk = header.copy(61);
                chunk.put64(1512, (index * 1024) as u64);
                chunk.put32(1496, bytes.len() as u32);
                chunk.0[TEXT..TEXT + 1024].fill(0);
                chunk.0[TEXT..TEXT + bytes.len()].copy_from_slice(bytes);
                channel.exchange(chunk)?;
            }
            channel.exchange(header.copy(62))?;
        }
        channel.exchange(finish)
    }
    pub(super) fn publish_video_moment(&self, s: MomentVideoSubmission<'_>, visibility: MomentVisibility) -> AppResult<SendReceipt> {
        let packet = self.dispatch_moment_content(&s.submission, &[s.cover], Some((s.video, s.bytes)), false, visibility)?;
        moment_receipt(s.submission.outbox_id, s.submission.attempt_id, s.submission.context_digest, &packet, false)
    }
    pub(super) fn publish_moment(
        &self,
        s: MomentImageSubmission<'_>,
        visibility: MomentVisibility,
    ) -> AppResult<SendReceipt> {
        let packet = self.dispatch_private_moment(&s.submission, s.images, false, visibility)?;
        moment_receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            &packet,
            false,
        )
    }
    pub(super) fn publish_private_moment(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        let packet = self.dispatch_private_moment(&s, &[], false, MomentVisibility::Private)?;
        moment_receipt(s.outbox_id, s.attempt_id, s.context_digest, &packet, false)
    }
    pub(super) fn publish_private_image_moment(
        &self,
        s: MomentImageSubmission<'_>,
    ) -> AppResult<SendReceipt> {
        if s.images.is_empty() {
            return Err(error("NATIVE_MOMENT_INVALID"));
        }
        let packet = self.dispatch_private_moment(
            &s.submission,
            s.images,
            false,
            MomentVisibility::Private,
        )?;
        moment_receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            &packet,
            false,
        )
    }
    pub(super) fn query_private_moment(&self, q: TextReceiptQuery<'_>) -> AppResult<SendReceipt> {
        if q.context.target_id != q.context.sender_id {
            return Err(error("NATIVE_MOMENT_INVALID"));
        }
        self.receipt_context(&self.account_id, q.context)?;
        let packet = self.query_moment_packet(&q.context.target_id, q.attempt_id)?;
        moment_receipt(q.outbox_id, q.attempt_id, q.context_digest, &packet, false)
    }
    pub(super) fn query_moment_packet(&self, target: &str, attempt: &str) -> AppResult<Packet> {
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut packet = channel_binding(&channel, 5);
        packet.text(TARGET, 128, target)?;
        packet.text(ATTEMPT, 64, attempt)?;
        channel.exchange(packet)
    }
    /// Constructs the complete native model and connects/cancels its event only.
    pub fn probe_private_moment(&self, text: &str) -> AppResult<(u32, u32)> {
        self.probe_private_image_moment(text, &[])
    }
    pub fn probe_private_image_moment(
        &self,
        text: &str,
        images: &[MomentImage<'_>],
    ) -> AppResult<(u32, u32)> {
        self.probe_moment(text, images, MomentVisibility::Private)
    }
    pub fn probe_moment(
        &self,
        text: &str,
        images: &[MomentImage<'_>],
        visibility: MomentVisibility,
    ) -> AppResult<(u32, u32)> {
        self.probe_moment_content(text, images, None, visibility)
    }
    pub fn probe_video_moment(&self, text: &str, cover: MomentImage<'_>, video: &VideoSendAsset,
        bytes: &[u8], visibility: MomentVisibility) -> AppResult<(u32, u32)> {
        self.probe_moment_content(text, &[cover], Some((video, bytes)), visibility)
    }
    fn probe_moment_content(&self, text: &str, images: &[MomentImage<'_>], video: Option<(&VideoSendAsset, &[u8])>,
        visibility: MomentVisibility) -> AppResult<(u32, u32)> {
        let context = self.resolve_send_context(&self.account_id, &self.context.sender_id)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = format!("moment-probe-{}", hex::encode(random));
        let digest = hex::encode(Sha256::digest(text.as_bytes()));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        let s = TextSubmission {
            outbox_id: &attempt,
            attempt_id: &attempt,
            request_digest: &digest,
            context_digest: &digest,
            context: &context,
            text,
            deadline_unix: now as i64 + 60,
        };
        let mut result = self.dispatch_moment_content(&s, images, video, true, visibility)?;
        let until = Instant::now() + Duration::from_secs(20);
        while result.u32(1492) == 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
            result = self.query_moment_packet(&context.target_id, &attempt)?;
        }
        moment_receipt(&attempt, &attempt, &digest, &result, true)?;
        Ok((result.u32(1492), result.u32(1500)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_staging_preserves_nine_ordered_images_empty_body_and_query_only_reconciliation() {
        let p = owned_script_provider(
            r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$stage=$null;$image=$null;$bytes=New-Object IO.MemoryStream;$images=0;$submits=0
function W32([int]$at,[uint32]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,4)}
function W64([int]$at,[uint64]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,8)}
while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if(!$r){exit};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);$err=0
if($op -notin @(1,50,54,55,56,53,5)){$err=13}
if($op -eq 50){$stage=$p.Clone();if([BitConverter]::ToUInt32($p,1496) -ne 9 -or [BitConverter]::ToUInt32($p,1500) -ne 2 -or [BitConverter]::ToUInt64($p,1520) -ne 0 -or [Text.Encoding]::ASCII.GetString($p,208,13) -ne 'owned-fixture'){$err=13}}
if($op -in @(54,55,56,53)){if(!$stage){$err=13}else{foreach($range in @(@(208,128),@(336,64),@(400,65),@(40,8))){for($k=0;$k -lt $range[1];$k++){if($p[$range[0]+$k] -ne $stage[$range[0]+$k]){$err=13}}}}}
if($op -eq 54){if($image -or [BitConverter]::ToUInt64($p,1512) -ne $images -or [BitConverter]::ToUInt32($p,1500) -ne 131074){$err=13};$image=$p.Clone();$bytes.SetLength(0)}
if($op -eq 55){$count=[BitConverter]::ToUInt32($p,1496);if(!$image -or [BitConverter]::ToUInt64($p,1512) -ne $bytes.Length -or $count -eq 0 -or $count -gt 1024){$err=13}else{$bytes.Write($p,465,$count)}}
if($op -eq 56){if(!$image -or [BitConverter]::ToUInt64($p,1512) -ne $images){$err=13}else{$sha=[Security.Cryptography.SHA256]::Create();$hash=([BitConverter]::ToString($sha.ComputeHash($bytes.ToArray()))).Replace('-','').ToLowerInvariant();$sha.Dispose();if($bytes.Length -ne [BitConverter]::ToUInt64($image,1520) -or $hash -ne [Text.Encoding]::ASCII.GetString($image,465,64)){$err=13}};$images++;$image=$null}
if($op -eq 53){$submits++;if($submits -ne 1 -or $images -ne 9 -or $image -or [BitConverter]::ToUInt64($p,1520) -ne 0){$err=13};W32 1492 1;W32 1496 21;W64 1520 0}
if($op -eq 5){if($submits -ne 1){$err=13};W32 1492 21;W32 1496 21;W64 1520 9007199254740993}
W32 12 $err;$o.Write($p,0,1528);$o.Flush()}"#,
        );
        let context = p.resolve_send_context("owned", "owned-fixture").unwrap();
        let (info, png) = werc_domain::image::canonical_png(include_bytes!(
            "../../../tests/fixtures/owned-image.png"
        ))
        .unwrap();
        let asset = ImageSendAsset {
            asset_id: "owned-image".into(),
            source_sha256: "b".repeat(64),
            png_sha256: hex::encode(Sha256::digest(&png)),
            width: info.width,
            height: info.height,
        };
        let images: Vec<_> = (0..9)
            .map(|_| MomentImage {
                asset: &asset,
                png: &png,
            })
            .collect();
        let digest = "a".repeat(64);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let receipt = p
            .submit_private_image_moment(MomentImageSubmission {
                submission: TextSubmission {
                    outbox_id: "owned-moment",
                    attempt_id: "owned-attempt",
                    request_digest: &digest,
                    context_digest: &digest,
                    context: &context,
                    text: "",
                    deadline_unix: now + 60,
                },
                images: &images,
            })
            .unwrap();
        assert_eq!(receipt.evidence, SendEvidence::NativeQueued);
        for _ in 0..2 {
            let receipt = p
                .reconcile_private_moment(
                    TextReceiptQuery {
                        outbox_id: "owned-moment",
                        attempt_id: "owned-attempt",
                        context_digest: &digest,
                        context: &context,
                    },
                    None,
                )
                .unwrap();
            assert_eq!(receipt.evidence, SendEvidence::ServerAccepted);
        }
    }
    #[test]
    fn private_staging_binds_full_utf8_content_and_receipt_query_never_resubmits() {
        let p = owned_script_provider(
            r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$stage=$null;$bytes=New-Object IO.MemoryStream;$submits=0
function W32([int]$at,[uint32]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,4)}
function W64([int]$at,[uint64]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,8)}
while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if(!$r){exit};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);$err=0
if($op -notin @(1,50,51,53,5)){$err=13}
if($op -eq 50){$stage=$p.Clone();if([BitConverter]::ToUInt32($p,1500) -ne 2 -or [BitConverter]::ToUInt64($p,1520) -ne 10000 -or [Text.Encoding]::ASCII.GetString($p,208,13) -ne 'owned-fixture'){$err=13}}
if($op -in @(51,53)){if(!$stage){$err=13}else{foreach($range in @(@(208,128),@(336,64),@(400,65),@(40,8),@(1500,4),@(1520,8))){for($k=0;$k -lt $range[1];$k++){if($p[$range[0]+$k] -ne $stage[$range[0]+$k]){$err=13}}}}}
if($op -eq 51){$count=[BitConverter]::ToUInt32($p,1496);if([BitConverter]::ToUInt64($p,1512) -ne $bytes.Length -or $count -eq 0 -or $count -gt 1024){$err=13}else{$bytes.Write($p,465,$count)}}
if($op -eq 53){$submits++;$sha=[Security.Cryptography.SHA256]::Create();$actual=([BitConverter]::ToString($sha.ComputeHash($bytes.ToArray()))).Replace('-','').ToLowerInvariant();$sha.Dispose();if($submits -ne 1 -or $bytes.Length -ne 10000 -or $actual -ne [Text.Encoding]::ASCII.GetString($stage,465,64)){$err=13};W32 1492 1;W32 1496 21;W64 1520 0}
if($op -eq 5){if($submits -ne 1){$err=13};W32 1492 21;W32 1496 21;W64 1520 9007199254740993}
W32 12 $err;$o.Write($p,0,1528);$o.Flush()}"#,
        );
        let context = p.resolve_send_context("owned", "owned-fixture").unwrap();
        let text = "中".repeat(3333) + "x";
        let digest = "a".repeat(64);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let receipt = p
            .submit_private_moment(TextSubmission {
                outbox_id: "owned-moment",
                attempt_id: "owned-attempt",
                request_digest: &digest,
                context_digest: &digest,
                context: &context,
                text: &text,
                deadline_unix: now + 60,
            })
            .unwrap();
        assert_eq!(receipt.evidence, SendEvidence::NativeQueued);
        for _ in 0..2 {
            let receipt = p
                .reconcile_private_moment(
                    TextReceiptQuery {
                        outbox_id: "owned-moment",
                        attempt_id: "owned-attempt",
                        context_digest: &digest,
                        context: &context,
                    },
                    None,
                )
                .unwrap();
            assert_eq!(receipt.evidence, SendEvidence::ServerAccepted);
        }
    }
    #[test]
    fn private_completion_requires_distinct_kind_and_nonzero_server_id() {
        let mut p = Packet::new(5);
        p.put32(1496, 21);
        p.put32(1492, 21);
        p.put64(1520, 9007199254740993);
        let receipt = moment_receipt("outbox", "attempt", "digest", &p, false).unwrap();
        assert_eq!(receipt.evidence, SendEvidence::ServerAccepted);
        assert_eq!(
            receipt.provider_reference.as_deref(),
            Some("native-moment:9007199254740993")
        );
        p.put64(1520, 0);
        assert_eq!(
            moment_receipt("o", "a", "d", &p, false).unwrap().evidence,
            SendEvidence::Unknown
        );
        p.put32(1496, 18);
        assert!(moment_receipt("o", "a", "d", &p, false).is_err());
        p.put32(1496, 20);
        p.put32(1492, 20);
        assert_eq!(
            moment_receipt("o", "a", "d", &p, true).unwrap().evidence,
            SendEvidence::Unknown
        );
        assert!(moment_receipt("o", "a", "d", &p, false).is_err());
    }
}
