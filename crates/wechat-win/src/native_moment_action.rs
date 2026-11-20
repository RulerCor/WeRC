//! Authenticated native lookup/like/comment. Polling cannot submit again.
use super::*;
fn action_receipt(q: TextReceiptQuery<'_>, packet: &Packet) -> AppResult<SendReceipt> {
    if packet.u32(1496) != 23 {
        return Err(error("NATIVE_MOMENT_RECEIPT_MISMATCH"));
    }
    Ok(SendReceipt {
        outbox_id: q.outbox_id.into(),
        attempt_id: q.attempt_id.into(),
        context_digest: q.context_digest.into(),
        evidence: match packet.u32(1492) {
            4 => SendEvidence::RejectedBeforeSubmission,
            1 => SendEvidence::NativeQueued,
            _ => SendEvidence::Unknown,
        },
        provider_reference: Some(format!("native-moment-action-state:{}", packet.u32(1500))),
    })
}
impl NativeFileHelperProvider {
    fn moment_action_packet(
        &self,
        s: &TextSubmission<'_>,
        sns_id: &str,
        action: u32,
        reply: Option<&MomentReplyTarget>,
    ) -> AppResult<Packet> {
        if s.context.target_id != s.context.sender_id
            || sns_id.parse::<u64>().ok().filter(|id| *id != 0).is_none()
        {
            return Err(error("NATIVE_MOMENT_INVALID"));
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
        let mut packet = channel_binding(&channel, if action == 0 { 59 } else { 58 });
        packet.text(TARGET, 128, &s.context.sender_id)?;
        packet.text(ATTEMPT, 64, s.attempt_id)?;
        packet.text(DIGEST, 65, s.request_digest)?;
        packet.text(TEXT, 1024, s.text)?;
        packet.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        packet.put64(
            1520,
            sns_id.parse().map_err(|_| error("NATIVE_MOMENT_INVALID"))?,
        );
        packet.put32(1500, action);
        if let Some(reply) = reply {
            packet.text(TEXT + 896, 128, &reply.username)?;
            packet.put32(
                1496,
                reply
                    .id
                    .as_deref()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| error("NATIVE_MOMENT_INVALID"))?,
            );
            packet.put64(
                1512,
                reply
                    .id64
                    .as_deref()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| error("NATIVE_MOMENT_INVALID"))?,
            );
        }
        channel.exchange(packet)
    }
    pub(super) fn dispatch_moment_action(
        &self,
        s: TextSubmission<'_>,
        action: &MomentAction,
    ) -> AppResult<SendReceipt> {
        if !action.valid(s.text) {
            return Err(error("NATIVE_MOMENT_INVALID"));
        }
        let packet = self.moment_action_packet(
            &s,
            &action.sns_id,
            match action.action {
                MomentActionKind::Like => 1,
                MomentActionKind::Comment => 2,
                MomentActionKind::Unlike => 3,
                MomentActionKind::Delete => 4,
                MomentActionKind::Reply => 5,
            },
            action.reply.as_ref(),
        )?;
        action_receipt(
            TextReceiptQuery {
                outbox_id: s.outbox_id,
                attempt_id: s.attempt_id,
                context_digest: s.context_digest,
                context: s.context,
            },
            &packet,
        )
    }
    pub(super) fn query_moment_action(&self, q: TextReceiptQuery<'_>) -> AppResult<SendReceipt> {
        if q.context.target_id != q.context.sender_id {
            return Err(error("NATIVE_MOMENT_INVALID"));
        }
        self.receipt_context(&self.account_id, q.context)?;
        let packet = self.query_moment_packet(&q.context.target_id, q.attempt_id)?;
        action_receipt(q, &packet)
    }
    /// Lookup only. Never invokes like or comment. Useful for exact-build validation.
    pub fn probe_moment_action(&self, sns_id: &str) -> AppResult<(u32, u32)> {
        let context = self.resolve_send_context(&self.account_id, &self.context.sender_id)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = format!("sns-lookup-{}", hex::encode(random));
        let digest = hex::encode(Sha256::digest(sns_id.as_bytes()));
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
            text: "",
            deadline_unix: now as i64 + 30,
        };
        let mut packet = self.moment_action_packet(&s, sns_id, 0, None)?;
        let until = Instant::now() + Duration::from_secs(35);
        loop {
            if packet.u32(1496) != 22 {
                return Err(error("NATIVE_MOMENT_RECEIPT_MISMATCH"));
            }
            let state = packet.u32(1500);
            if matches!(state, 5..=9) {
                return Ok((packet.u32(1492), state));
            }
            if Instant::now() >= until {
                return Err(error("NATIVE_MOMENT_TIMEOUT"));
            }
            std::thread::sleep(Duration::from_millis(250));
            packet = self.query_moment_packet(&context.sender_id, &attempt)?;
        }
    }
}
