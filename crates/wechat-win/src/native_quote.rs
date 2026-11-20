use super::*;
impl NativeFileHelperProvider {
    pub(super) fn dispatch_quote(
        &self,
        s: &TextSubmission<'_>,
        source: &QuoteSource,
        probe: bool,
    ) -> AppResult<(u32, u32)> {
        if !source.valid()
            || hex::encode(Sha256::digest(source.text.as_bytes())) != source.sha256
            || !valid_send_text(s.text)
        {
            return Err(error("NATIVE_QUOTE_INVALID"));
        }
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.target_id)? {
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
        let mut packet = channel_binding(&channel, 45);
        packet.text(TARGET, 128, &s.context.target_id)?;
        packet.text(ATTEMPT, 64, s.attempt_id)?;
        packet.text(DIGEST, 65, s.request_digest)?;
        packet.text(TEXT, 65, &source.sha256)?;
        packet.text(
            TEXT + 65,
            65,
            &hex::encode(Sha256::digest(s.text.as_bytes())),
        )?;
        packet.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        packet.put64(
            1520,
            source
                .server_id
                .parse()
                .map_err(|_| error("NATIVE_QUOTE_INVALID"))?,
        );
        packet.put32(1500, s.text.len() as u32);
        let finish = packet.copy(if probe { 46 } else { 47 });
        channel.exchange(packet)?;
        for (i, bytes) in s.text.as_bytes().chunks(1024).enumerate() {
            let mut chunk = finish.copy(48);
            chunk.put64(1512, (i * 1024) as u64);
            chunk.put32(1496, bytes.len() as u32);
            chunk.0[TEXT..TEXT + 1024].fill(0);
            chunk.0[TEXT..TEXT + bytes.len()].copy_from_slice(bytes);
            channel.exchange(chunk)?;
        }
        let result = channel.exchange(finish)?;
        Ok((result.u32(1492), result.u32(1500)))
    }
    /// Trusted local probe only. No native send is scheduled by this operation.
    pub fn probe_quote_to(
        &self,
        target: &str,
        original_id: u64,
        original_text: &str,
        reply: &str,
    ) -> AppResult<(u32, u32)> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        let source = QuoteSource {
            server_id: original_id.to_string(),
            text: original_text.into(),
            sha256: hex::encode(Sha256::digest(original_text.as_bytes())),
        };
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = format!("quote-probe-{}", hex::encode(random));
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&(target, &source, reply)).map_err(|_| AppError::internal())?,
        ));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        let submission = TextSubmission {
            outbox_id: &attempt,
            attempt_id: &attempt,
            request_digest: &digest,
            context_digest: &digest,
            context: &context,
            text: reply,
            deadline_unix: now as i64 + 60,
        };
        let (mut code, mut stage) = self.dispatch_quote(&submission, &source, true)?;
        let until = Instant::now() + Duration::from_secs(20);
        while code == 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            let mut p = channel_binding(&channel, 5);
            p.text(TARGET, 128, target)?;
            p.text(ATTEMPT, 64, &attempt)?;
            let result = channel.exchange(p)?;
            code = result.u32(1492);
            stage = result.u32(1500);
        }
        Ok((code, stage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quote_protocol_binds_source_reply_and_full_64bit_id_without_plain_fallback() {
        let p = owned_script_provider(
            r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$stage=$null;$bytes=New-Object System.IO.MemoryStream;while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if(!$r){exit};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);$err=0;if($op -eq 45){$stage=$p.Clone();if([BitConverter]::ToUInt64($p,1520) -ne [uint64]9007199254740993 -or [BitConverter]::ToUInt32($p,1500) -ne 10000){$err=13};$sha=[Security.Cryptography.SHA256]::Create();$expected=([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes('owned source')))).Replace('-','').ToLowerInvariant();$sha.Dispose();if([Text.Encoding]::ASCII.GetString($p,465,64) -ne $expected){$err=13}};if($op -in @(47,48)){if(!$stage){$err=13}else{foreach($range in @(@(208,128),@(336,64),@(400,65),@(40,8),@(1500,4),@(1520,8))){for($k=0;$k -lt $range[1];$k++){if($p[$range[0]+$k] -ne $stage[$range[0]+$k]){$err=13}}}}};if($op -eq 48){$count=[BitConverter]::ToUInt32($p,1496);if([BitConverter]::ToUInt64($p,1512) -ne $bytes.Length -or $count -eq 0 -or $count -gt 1024){$err=13}else{$bytes.Write($p,465,$count)}};if($op -eq 47){$sha=[Security.Cryptography.SHA256]::Create();$actual=([BitConverter]::ToString($sha.ComputeHash($bytes.ToArray()))).Replace('-','').ToLowerInvariant();$sha.Dispose();if($bytes.Length -ne 10000 -or $actual -ne [Text.Encoding]::ASCII.GetString($stage,530,64)){$err=13};[Array]::Copy([BitConverter]::GetBytes([uint32]2),0,$p,1492,4)};if($op -in @(3,39,43)){$err=13};[Array]::Copy([BitConverter]::GetBytes([uint32]$err),0,$p,12,4);$o.Write($p,0,1528);$o.Flush()}"#,
        );
        let context = p.resolve_send_context("owned", "filehelper").unwrap();
        let text = "中".repeat(3333) + "x";
        let mut source = QuoteSource {
            server_id: "9007199254740993".into(),
            text: "owned source".into(),
            sha256: "0".repeat(64),
        };
        let hash = "a".repeat(64);
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 60;
        let s = || TextSubmission {
            outbox_id: "owned-quote-outbox",
            attempt_id: "owned-quote-attempt",
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: &text,
            deadline_unix: deadline,
        };
        assert!(
            p.submit_quote(QuoteSubmission {
                submission: s(),
                source: &source
            })
            .is_err()
        );
        source.sha256 = hex::encode(Sha256::digest(source.text.as_bytes()));
        let receipt = p
            .submit_quote(QuoteSubmission {
                submission: s(),
                source: &source,
            })
            .unwrap();
        assert_eq!(receipt.evidence, SendEvidence::NativeQueued);
        assert_eq!(receipt.attempt_id, "owned-quote-attempt");
    }
}
