use super::*;
use werc_domain::wechat_settings::*;
impl NativeFileHelperProvider {
    pub fn settings_read(&self, account: &str) -> AppResult<SettingsSnapshot> {
        self.require_open()?;
        if account != self.account_id {
            return Err(error("ACCOUNT_MISMATCH"));
        }
        let mut c = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        c.check_identity()?;
        let mut p = channel_binding(&c, 70);
        p.put64(40, now()? + 20);
        let p = c.exchange(p)?;
        let build = NativeBuild {
            version: p.string(TEXT, 1024)?,
            architecture: "x86_64".into(),
            core_sha256: p.string(DIGEST, 65)?,
        };
        let mask = crate::native_profiles::settings_mask(
            &build.version,
            &build.architecture,
            &build.core_sha256,
        );
        if mask == 0 || p.u32(1496) != mask || p.u32(1500) & !mask != 0 {
            return Err(error("WECHAT_SETTINGS_UNAVAILABLE"));
        }
        Ok(SettingsSnapshot {
            build,
            native_epoch: self.context.epoch.clone(),
            values: SettingKey::ALL
                .into_iter()
                .filter(|k| mask & (1 << (k.native_id() - 1)) != 0)
                .map(|key| SettingValue {
                    key,
                    value: p.u32(1500) & (1 << (key.native_id() - 1)) != 0,
                })
                .collect(),
            source: "wechat_native_typed_getter".into(),
        })
    }
    pub fn settings_apply(
        &self,
        account: &str,
        t: &SettingTarget,
        epoch: &str,
    ) -> AppResult<SettingResult> {
        self.require_open()?;
        if account != self.account_id || epoch != self.context.epoch {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        let mut c = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        c.check_identity()?;
        let mut p = channel_binding(&c, 71);
        p.put64(40, now()? + 20);
        p.put32(1500, t.key.native_id());
        p.put32(1496, u32::from(t.value));
        p.put64(1512, t.expected_value.map(u64::from).unwrap_or(2));
        let p = c.exchange(p)?;
        if p.u64(1512) > 1 || p.u64(1520) > 1 {
            return Err(error("NATIVE_PROTOCOL_INVALID"));
        }
        let status = match p.u32(1492) {
            1 => ChangeStatus::Verified,
            2 => ChangeStatus::Unchanged,
            3 => ChangeStatus::Conflict,
            4 => ChangeStatus::Unknown,
            _ => return Err(error("NATIVE_PROTOCOL_INVALID")),
        };
        let before = p.u64(1520) != 0;
        let observed = p.u64(1512) != 0;
        if matches!(status, ChangeStatus::Verified | ChangeStatus::Unchanged) && observed != t.value
        {
            return Err(error("NATIVE_PROTOCOL_INVALID"));
        }
        Ok(SettingResult {
            status,
            before: Some(before),
            requested: t.value,
            observed: Some(observed),
            error_code: if status == ChangeStatus::Unknown {
                Some("SETTINGS_READBACK_MISMATCH".into())
            } else {
                None
            },
        })
    }
}
fn now() -> AppResult<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::internal())?
        .as_secs())
}
