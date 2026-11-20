//! Session-scoped auxiliary database keys. Only DPAPI envelopes are retained.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct AuxiliaryKeys(Mutex<BTreeMap<String, Vec<u8>>>);

impl AuxiliaryKeys {
    // Callers hold the session read lease and pin the exact database file for
    // this entire operation. A reconnect drops this cache with the old session.
    pub(super) fn acquire(
        &self,
        account: &str,
        context: &SendContext,
        pid: u32,
        path: &Path,
        fingerprint: &str,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let database = path.to_str().ok_or_else(AppError::internal)?;
        let salt = crate::key_acquisition::database_salt(database)?;
        let binding = serde_json::to_vec(&(account, context, pid, fingerprint, salt))
            .map_err(|_| AppError::internal())?;
        let tag = format!(
            "WeRC/auxiliary-key/v1/{}",
            hex::encode(Sha256::digest(binding))
        );
        let mut keys = self.0.lock().map_err(|_| AppError::internal())?;
        if let Some(envelope) = keys.get(&tag) {
            // The worker still authenticates database pages on every read.
            // A hit skips only the repeated process-wide memory scan.
            return crate::keys::unprotect(&tag, envelope).map(Zeroizing::new);
        }
        let installations = crate::discovery::discover()?.installations;
        let clients: Vec<_> = installations
            .iter()
            .filter(|i| i.process_ids.contains(&pid) && i.sha256 == context.installation_sha256)
            .collect();
        if clients.len() != 1 {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "当前微信进程身份已变化",
            ));
        }
        let key = crate::key_acquisition::acquire(clients[0], pid, database)?;
        if crate::key_acquisition::database_salt(database)? != salt {
            return Err(AppError::new(
                "SOURCE_CHANGED",
                "取钥期间数据库密钥标识已变化，请重新读取",
            ));
        }
        let envelope = crate::keys::protect(&tag, key.key.as_ref())?;
        // There are four auxiliary databases today. Bound old identity/salt
        // entries without imposing a lifetime on an actively browsed source.
        if keys.len() >= 16 {
            keys.clear();
        }
        keys.insert(tag, envelope);
        Ok(Zeroizing::new(key.key.to_vec()))
    }
}
