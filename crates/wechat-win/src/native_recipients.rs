//! Recipient leases are independent of read-window lifetime and never send.
use super::*;

pub(super) struct RecipientLease {
    context: SendContext,
    expires: Instant,
    deadline: u64,
    _sources: Vec<crate::live_source::LiveSourcePin>,
}

fn valid_target(target: &str) -> bool {
    !target.is_empty()
        && target.len() < 128
        && target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'@'))
}
fn now() -> AppResult<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
        .as_secs())
}
fn summary(context: &SendContext, deadline: Option<u64>) -> PreparedRecipient {
    PreparedRecipient {
        target_id: context.target_id.clone(),
        display_name: context.target_display_name.clone(),
        expires_at: deadline.map(|d| d as i64),
    }
}
impl NativeFileHelperProvider {
    fn builtin_context(&self, target: &str) -> Option<SendContext> {
        if target == "filehelper" {
            return Some(self.context.clone());
        }
        if target == self.context.sender_id {
            let mut context = self.context.clone();
            context.target_id = target.into();
            context.target_display_name = "当前登录账号（自己）".into();
            return Some(context);
        }
        None
    }
    pub(crate) fn prepare_verified_recipient(
        &self,
        account: &str,
        target: &str,
        sources: Vec<crate::live_source::LiveSourcePin>,
    ) -> AppResult<PreparedRecipient> {
        if account != self.account_id
            || !valid_target(target)
            || sources.is_empty()
            || sources.len() > 16
        {
            return Err(error("NATIVE_TARGET_NOT_VERIFIED"));
        }
        self.prepare_target_lease(account, target, sources, false)
    }
    pub(crate) fn prepare_client_target(
        &self,
        account: &str,
        target: &str,
    ) -> AppResult<PreparedRecipient> {
        if account != self.account_id || !valid_target(target) {
            return Err(error("NATIVE_TARGET_NOT_VERIFIED"));
        }
        if let Ok(context) = self.recipient_context(account, target) {
            let deadline = self
                .recipient_list(account)?
                .into_iter()
                .find(|r| r.target_id == target)
                .and_then(|r| r.expires_at);
            return Ok(summary(&context, deadline.map(|d| d as u64)));
        }
        self.prepare_target_lease(account, target, Vec::new(), true)
    }
    fn prepare_target_lease(
        &self,
        _account: &str,
        target: &str,
        sources: Vec<crate::live_source::LiveSourcePin>,
        reuse: bool,
    ) -> AppResult<PreparedRecipient> {
        self.require_open()?;
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        if let Some(context) = self.builtin_context(target) {
            return Ok(summary(&context, None));
        }
        let time = now()?;
        let mut leases = self.recipients.lock().map_err(|_| AppError::internal())?;
        leases.retain(|r| r.expires > Instant::now() && r.deadline > time);
        if reuse && let Some(existing) = leases.iter().find(|r| r.context.target_id == target) {
            return Ok(summary(&existing.context, Some(existing.deadline)));
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let mut context = self.context.clone();
        context.target_id = target.into();
        context.target_display_name = target.into(); // Never invent a verified nickname.
        context.epoch = format!("{}:target:{}", self.context.epoch, hex::encode(random));
        let deadline = time + 300;
        let mut packet = channel_binding(&channel, 37);
        packet.text(TARGET, 128, target)?;
        packet.put64(40, deadline);
        channel.exchange(packet)?;
        leases.retain(|r| r.context.target_id != target);
        let result = summary(&context, Some(deadline));
        leases.push(RecipientLease {
            context,
            expires: Instant::now() + Duration::from_secs(300),
            deadline,
            _sources: sources,
        });
        Ok(result)
    }
    pub(super) fn recipient_context(&self, account: &str, target: &str) -> AppResult<SendContext> {
        self.require_open()?;
        if account != self.account_id || !valid_target(target) {
            return Err(error("NATIVE_TARGET_NOT_VERIFIED"));
        }
        self.transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?
            .check_identity()?;
        if let Some(context) = self.builtin_context(target) {
            return Ok(context);
        }
        let time = now()?;
        let mut leases = self.recipients.lock().map_err(|_| AppError::internal())?;
        leases.retain(|r| r.expires > Instant::now() && r.deadline > time);
        leases
            .iter()
            .find(|r| r.context.target_id == target)
            .map(|r| r.context.clone())
            .ok_or_else(|| error("NATIVE_TARGET_NOT_VERIFIED"))
    }
    pub(super) fn recipient_list(&self, account: &str) -> AppResult<Vec<PreparedRecipient>> {
        self.recipient_context(account, "filehelper")?;
        let mut result = vec![summary(&self.context, None)];
        if let Some(context) = self.builtin_context(&self.context.sender_id) {
            result.push(summary(&context, None));
        }
        let time = now()?;
        let mut leases = self.recipients.lock().map_err(|_| AppError::internal())?;
        leases.retain(|r| r.expires > Instant::now() && r.deadline > time);
        result.extend(leases.iter().map(|r| summary(&r.context, Some(r.deadline))));
        Ok(result)
    }
    pub(super) fn send_deadline(
        &self,
        context: &SendContext,
        requested: i64,
        time: u64,
    ) -> AppResult<u64> {
        if requested <= time as i64 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        if let Some(builtin) = self.builtin_context(&context.target_id) {
            if builtin != *context {
                return Err(error("NATIVE_CONTEXT_CHANGED"));
            }
            return Ok((requested as u64).min(time + 300));
        }
        let leases = self.recipients.lock().map_err(|_| AppError::internal())?;
        let lease = leases
            .iter()
            .find(|r| r.context == *context && r.expires > Instant::now() && r.deadline > time)
            .ok_or_else(|| error("NATIVE_TARGET_NOT_VERIFIED"))?;
        Ok((requested as u64).min(time + 300).min(lease.deadline))
    }
    pub(super) fn receipt_context(&self, account: &str, context: &SendContext) -> AppResult<()> {
        self.require_open()?;
        if account != self.account_id || !valid_target(&context.target_id) {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        let mut expected = self.context.clone();
        if let Some(builtin) = self.builtin_context(&context.target_id) {
            expected = builtin;
        } else {
            let prefix = format!("{}:target:", self.context.epoch);
            let valid = context
                .epoch
                .strip_prefix(&prefix)
                .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()));
            if !valid {
                return Err(error("NATIVE_CONTEXT_CHANGED"));
            }
            expected.target_id = context.target_id.clone();
            expected.target_display_name = context.target_id.clone();
            expected.epoch = context.epoch.clone();
        }
        if expected != *context {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        self.transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?
            .check_identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider() -> NativeFileHelperProvider {
        owned_script_provider(
            r#"$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if($r -eq 0){exit 0};$n+=$r};$o.Write($p,0,1528);$o.Flush()}"#,
        )
    }
    #[test]
    fn preparation_pins_sources_expires_and_never_revives_old_approval() {
        let p = provider();
        let directory = std::env::temp_dir().join(format!(
            "werc-owned-recipient-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("message_0.db");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let pins = || vec![crate::live_source::LiveSourcePin::open(&path).unwrap()];
        let target = "owned_long_group_identifier_12345@chatroom";
        assert!(p.resolve_send_context("owned", target).is_err());
        assert!(
            p.prepare_verified_recipient("other", target, pins())
                .is_err()
        );
        assert!(
            p.prepare_verified_recipient("owned", "bad target", pins())
                .is_err()
        );
        assert!(
            p.prepare_verified_recipient("owned", target, vec![])
                .is_err()
        );
        let prepared = p
            .prepare_verified_recipient("owned", target, pins())
            .unwrap();
        assert_eq!(prepared.target_id, target);
        let first = p.resolve_send_context("owned", target).unwrap();
        assert_eq!(first.target_id, target);
        assert_ne!(first.epoch, p.context.epoch);
        assert_eq!(p.prepared_recipients("owned").unwrap().len(), 3);
        assert!(std::fs::rename(&path, directory.join("old.db")).is_err());
        p.prepare_verified_recipient("owned", target, pins())
            .unwrap();
        let second = p.resolve_send_context("owned", target).unwrap();
        assert_ne!(first, second);
        assert!(
            p.send_deadline(&first, now().unwrap() as i64 + 60, now().unwrap())
                .is_err()
        );
        assert!(p.receipt_context("owned", &first).is_ok());
        let mut changed = first.clone();
        changed.sender_id.push('x');
        assert!(p.receipt_context("owned", &changed).is_err());
        let mut leases = p.recipients.lock().unwrap();
        leases[0].expires = Instant::now() - Duration::from_secs(1);
        drop(leases);
        assert!(p.resolve_send_context("owned", target).is_err());
        assert_eq!(p.prepared_recipients("owned").unwrap().len(), 2);
        assert!(p.receipt_context("owned", &second).is_ok());
        std::fs::rename(&path, directory.join("old.db")).unwrap();
        p.request_disconnect();
        assert!(p.receipt_context("owned", &second).is_err());
        drop(p);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn builtin_self_has_exact_identity_and_never_mutates_receive_context() {
        let p = provider();
        let context = p.resolve_send_context("owned", "owned-fixture").unwrap();
        assert_eq!(context.sender_id, context.target_id);
        assert_eq!(p.context.target_id, "filehelper");
        assert!(p.resolve_send_context("other", "owned-fixture").is_err());
        assert!(
            p.resolve_send_context("owned", "owned-fixture-extra")
                .is_err()
        );
        let mut forged = context.clone();
        forged.epoch.push('x');
        assert!(p.receipt_context("owned", &forged).is_err());
        assert!(p.receipt_context("owned", &context).is_ok());
    }
    #[test]
    fn client_exact_target_reuses_live_lease_without_local_view_or_send() {
        let p = provider();
        let target = "owned-bot-target@chatroom";
        assert!(p.prepare_client_target("other", target).is_err());
        assert!(p.prepare_client_target("owned", "昵称不是账号").is_err());
        let first = p.prepare_client_target("owned", target).unwrap();
        let context = p.recipient_context("owned", target).unwrap();
        let again = p.prepare_client_target("owned", target).unwrap();
        assert_eq!(first.expires_at, again.expires_at);
        assert_eq!(context, p.recipient_context("owned", target).unwrap());
        assert!(p.recipients.lock().unwrap()[0]._sources.is_empty());
        assert_eq!(p.context.target_id, "filehelper");
    }
}
