use super::*;
impl NativeFileHelperProvider {
    pub(super) fn dispatch_mentions(
        &self,
        s: &TextSubmission<'_>,
        members: &[String],
        probe: bool,
    ) -> AppResult<u32> {
        self.require_feature(NativeFeature::Mentions)?;
        if !werc_domain::mentions::valid_members(&s.context.target_id, members)
            || s.text.trim().is_empty()
            || s.text.len() > MAX_TEXT_BYTES
            || s.text.contains('\0')
        {
            return Err(error("NATIVE_MENTION_INVALID"));
        }
        if s.text.len() >= 1024 {
            return self.dispatch_long_text(s, members, probe);
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
        let mut packet = channel_binding(&channel, 38);
        packet.text(TARGET, 128, &s.context.target_id)?;
        packet.text(ATTEMPT, 64, s.attempt_id)?;
        packet.text(DIGEST, 65, s.request_digest)?;
        packet.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        let csv = Zeroizing::new(members.join(","));
        packet.text(TEXT, 1024, &csv)?;
        let mut submit = packet.copy(if probe { 40 } else { 39 });
        submit.text(TEXT, 1024, s.text)?;
        channel.exchange(packet)?; // Stage is never a native submit.
        let result = channel.exchange(submit)?;
        Ok(result.u32(1492))
    }
    /// Trusted local verification: constructs/releases a native text object only.
    /// Requires an already authenticated/prepared group and exact member IDs.
    pub fn probe_mentions_to(
        &self,
        target: &str,
        text: &str,
        members: &[String],
    ) -> AppResult<bool> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = format!("mention-probe-{}", hex::encode(random));
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&(target, text, members)).map_err(|_| AppError::internal())?,
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
            text,
            deadline_unix: now as i64 + 60,
        };
        let mut code = self.dispatch_mentions(&submission, members, true)?;
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
        Ok(code == if text.len() >= 1024 { 16 } else { 15 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_stages_exact_selection_and_never_falls_back_to_plain_text() {
        let p = owned_script_provider(
            r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$staged=$null;$sent=0;while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if(!$r){exit};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);$err=0;if($op -eq 38){$str=[Text.Encoding]::UTF8.GetString($p,465,1024).Split([char]0)[0];if($str -ne 'member-a,member_long_1234567890'){$err=13};$staged=$p.Clone()};if($op -eq 39){$sent++;if(!$staged -or $sent -ne 1){$err=13};foreach($range in @(@(208,128),@(336,64),@(400,65),@(40,8))){for($k=0;$k -lt $range[1];$k++){if($p[$range[0]+$k] -ne $staged[$range[0]+$k]){$err=13}}};if([Text.Encoding]::UTF8.GetString($p,465,1024).Split([char]0)[0] -ne 'owned automation mention'){$err=13};[Array]::Copy([BitConverter]::GetBytes([uint32]2),0,$p,1492,4)};if($op -eq 3){$err=13};[Array]::Copy([BitConverter]::GetBytes([uint32]$err),0,$p,12,4);$o.Write($p,0,1528);$o.Flush()}"#,
        );
        let dir = std::env::temp_dir().join(format!(
            "werc-owned-mentions-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("message_0.db");
        std::fs::write(&file, [0u8; 4096]).unwrap();
        let target = "owned-group@chatroom";
        p.prepare_verified_recipient(
            "owned",
            target,
            vec![crate::live_source::LiveSourcePin::open(&file).unwrap()],
        )
        .unwrap();
        let context = p.resolve_send_context("owned", target).unwrap();
        let hash = "a".repeat(64);
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 60;
        let submission = || TextSubmission {
            outbox_id: "owned-outbox",
            attempt_id: "owned-mention-attempt",
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: "owned automation mention",
            deadline_unix: deadline,
        };
        let members = vec!["member-a".into(), "member_long_1234567890".into()];
        assert!(
            p.submit_mentions(MentionSubmission {
                submission: submission(),
                members: &["@all".into()]
            })
            .is_err()
        );
        let receipt = p
            .submit_mentions(MentionSubmission {
                submission: submission(),
                members: &members,
            })
            .unwrap();
        assert_eq!(receipt.evidence, SendEvidence::NativeQueued);
        assert_eq!(receipt.attempt_id, "owned-mention-attempt");
        drop(p);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
