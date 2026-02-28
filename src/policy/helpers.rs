use std::fs;
use std::path::Path;

use crate::model::{ReasonCode, Zone};
use crate::utils::path_to_string;

pub(super) fn deny_reason_for_zone(zone: Zone) -> ReasonCode {
    match zone {
        Zone::SystemCritical => ReasonCode::DenySystemCritical,
        Zone::Secrets => ReasonCode::DenySecrets,
        Zone::UserData => ReasonCode::DenyUserDataWrite,
        Zone::Workspace | Zone::Shared => ReasonCode::DenyInvalidRequest,
    }
}

pub(super) fn is_tmp_path(path: &Path) -> bool {
    let normalized = path_to_string(path);
    normalized.starts_with("/tmp/")
        || normalized.starts_with("/var/tmp/")
        || normalized.starts_with("/dev/shm/")
        || normalized.contains("/exec-layer/")
}

#[cfg(target_family = "unix")]
pub(super) fn owner_uid(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    if let Ok(meta) = fs::metadata(path) {
        return Some(meta.uid());
    }
    if let Some(parent) = path.parent() {
        if let Ok(meta) = fs::metadata(parent) {
            return Some(meta.uid());
        }
    }
    None
}

#[cfg(not(target_family = "unix"))]
pub(super) fn owner_uid(_path: &Path) -> Option<u32> {
    None
}
