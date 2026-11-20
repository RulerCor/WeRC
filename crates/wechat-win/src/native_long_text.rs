use super::*;

impl NativeFileHelperProvider {
    pub(super) fn dispatch_long_text(
        &self,
        s: &TextSubmission<'_>,
        members: &[String],
        probe: bool,
    ) -> AppResult<u32> {
        if !members.is_empty() {
            self.require_feature(NativeFeature::Mentions)?;
        }
        if !valid_send_text(s.text)
            || s.text.len() < 1024
            || (!members.is_empty()
                && !werc_domain::mentions::valid_members(&s.context.target_id, members))
        {
            return Err(error("NATIVE_TEXT_INVALID"));
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
        let mut begin = channel_binding(&channel, 41);
        begin.text(TARGET, 128, &s.context.target_id)?;
        begin.text(ATTEMPT, 64, s.attempt_id)?;
        begin.text(DIGEST, 65, s.request_digest)?;
        begin.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        if !members.is_empty() {
            let mut selection = begin.copy(38);
            let csv = Zeroizing::new(members.join(","));
            selection.text(TEXT, 1024, &csv)?;
            channel.exchange(selection)?;
        }
        begin.put64(1520, s.text.len() as u64);
        begin.put32(1500, u32::from(!members.is_empty()));
        begin.text(TEXT, 1024, &hex::encode(Sha256::digest(s.text.as_bytes())))?;
        let mut chunk_template = begin.copy(42);
        let mut submit = begin.copy(if probe { 44 } else { 43 });
        chunk_template.0[TEXT..TEXT + 1024].fill(0);
        submit.0[TEXT..TEXT + 1024].fill(0);
        channel.exchange(begin)?;
        for (index, chunk) in s.text.as_bytes().chunks(1024).enumerate() {
            let mut packet = chunk_template.copy(42);
            packet.0[TEXT..TEXT + chunk.len()].copy_from_slice(chunk);
            packet.put64(1512, (index * 1024) as u64);
            packet.put32(1496, chunk.len() as u32);
            channel.exchange(packet)?;
        }
        Ok(channel.exchange(submit)?.u32(1492))
    }

    /// Constructs and releases both request and message strings; never sends.
    pub fn probe_long_text_to(
        &self,
        target: &str,
        text: &str,
        members: &[String],
    ) -> AppResult<bool> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = format!("long-probe-{}", hex::encode(random));
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&(target, text, members)).map_err(|_| AppError::internal())?,
        ));
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
        let mut code = self.dispatch_long_text(&s, members, true)?;
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
            code = channel.exchange(p)?.u32(1492);
        }
        Ok(code == 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn protocol_provider(fail_chunk: bool) -> NativeFileHelperProvider {
        let script = r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$start=$null;$bytes=New-Object IO.MemoryStream;$count=0;$members=$false;
while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if(!$r){exit};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);$err=0;
if($op -eq 38){$members=[Text.Encoding]::UTF8.GetString($p,465,1024).Split([char]0)[0] -eq 'member-a'};
if($op -eq 41){$start=$p.Clone();if([BitConverter]::ToUInt64($p,1520) -ne 10000){$err=13};if([BitConverter]::ToUInt32($p,1500) -eq 1 -and !$members){$err=13}};
if($op -eq 42){$offset=[BitConverter]::ToUInt64($p,1512);$size=[BitConverter]::ToUInt32($p,1496);if(!$start -or $offset -ne $bytes.Length -or $size -gt 1024){$err=13};foreach($range in @(@(208,257),@(40,8),@(1500,4),@(1520,8))){for($k=0;$k -lt $range[1];$k++){if($p[$range[0]+$k] -ne $start[$range[0]+$k]){$err=13}}};$bytes.Write($p,465,$size);if(FAIL_CHUNK -and $offset -eq 1024){$err=13}};
if($op -eq 43){$count++;if($count -ne 1 -or $bytes.Length -ne 10000){$err=13};$hash=[BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($bytes.ToArray())).Replace('-','').ToLowerInvariant();if($hash -ne [Text.Encoding]::ASCII.GetString($start,465,64)){$err=13};$utf8=New-Object Text.UTF8Encoding($false,$true);$text=$utf8.GetString($bytes.ToArray());if(!$text.StartsWith('automation-')){$err=13};[Array]::Copy([BitConverter]::GetBytes([uint32]2),0,$p,1492,4)};
if($op -eq 3 -or $op -eq 39){$err=13};[Array]::Copy([BitConverter]::GetBytes([uint32]$err),0,$p,12,4);$o.Write($p,0,1528);$o.Flush()}"#;
        owned_script_provider(
            &script.replace("FAIL_CHUNK", if fail_chunk { "$true" } else { "$false" }),
        )
    }
    #[test]
    fn long_utf8_protocol_binds_all_chunks_and_has_no_short_fallback() {
        for with_members in [false, true] {
            let p = protocol_provider(false);
            let dir =
                std::env::temp_dir().join(format!("werc-long-{}", hex::encode(getrandom_array())));
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join("message_0.db");
            std::fs::write(&path, [0u8; 4096]).unwrap();
            let target = if with_members {
                "owned-long@chatroom"
            } else {
                "filehelper"
            };
            if with_members {
                p.prepare_verified_recipient(
                    "owned",
                    target,
                    vec![crate::live_source::LiveSourcePin::open(&path).unwrap()],
                )
                .unwrap();
            }
            let context = p.resolve_send_context("owned", target).unwrap();
            let mut text = format!("automation-{}", "中".repeat(3329));
            text.push_str(&"x".repeat(10000 - text.len()));
            let hash = "a".repeat(64);
            let deadline = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                + 60;
            let s = TextSubmission {
                outbox_id: "owned",
                attempt_id: "owned-long-protocol",
                request_digest: &hash,
                context_digest: &hash,
                context: &context,
                text: &text,
                deadline_unix: deadline,
            };
            let receipt = if with_members {
                p.submit_mentions(MentionSubmission {
                    submission: s,
                    members: &["member-a".into()],
                })
                .unwrap()
            } else {
                p.submit_text(s).unwrap()
            };
            assert_eq!(receipt.evidence, SendEvidence::NativeQueued);
            drop(p);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    fn getrandom_array() -> [u8; 16] {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn failed_chunk_stops_before_submit_and_oversized_body_never_stages() {
        let p = protocol_provider(true);
        let context = p.resolve_send_context("owned", "filehelper").unwrap();
        let hash = "a".repeat(64);
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 60;
        for text in [
            format!("automation-{}", "x".repeat(9989)),
            "x".repeat(10001),
        ] {
            let s = TextSubmission {
                outbox_id: "owned",
                attempt_id: "owned-long-failed",
                request_digest: &hash,
                context_digest: &hash,
                context: &context,
                text: &text,
                deadline_unix: deadline,
            };
            assert!(p.submit_text(s).is_err());
        }
    }
}
